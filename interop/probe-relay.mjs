// Blind-relay probe: peeroxide-dht relay/server vs the JS hyperdht + blind-relay on a local testnet (127.0.0.1).
// Run: TOWNJUMP_JS_NM=<node_modules> CARGO_TARGET_DIR=<target> node interop/probe-relay.mjs [scenario names]
// Binaries are NOT built here: PROBE_BIN / PROBE_BIN_RELAY or <CARGO_TARGET_DIR>/debug/examples/{probe,probe_relay}.
// Output: `<name> → ok` / `<name> → fail: <reason>`; timeout per scenario 45 s; logs in <root>/.tmp/probe-relay.
import crypto from 'node:crypto'
import fs from 'node:fs'
import path from 'node:path'
import { createRequire } from 'node:module'
import { commandFor, ROOT_DIR } from './lib/impl.mjs'
import { Proc, sleep } from './lib/proc.mjs'
import { startTestnet } from './lib/testnet.mjs'
import { rsScenarios } from './probe-relay-rs.mjs'

const require = createRequire(import.meta.url)
const nm = require('./js/nm.cjs')
const HyperDHT = nm.req('hyperdht')
const Holepuncher = nm.req('hyperdht/lib/holepuncher')
const RelayServer = nm.req('blind-relay').Server

const BYTES = 1 << 20
const CHUNK = 64 * 1024
const SCENARIO_MS = 45_000
const LOG_DIR = path.join(ROOT_DIR, '.tmp/probe-relay')
const TARGET = process.env.CARGO_TARGET_DIR ?? path.resolve(ROOT_DIR, '../../target-relay-upgrade')

const seed = (tag) => Buffer.alloc(32, tag)
const keyOf = (tag) => HyperDHT.keyPair(seed(tag))
const pattern = (n, from = 0) => Buffer.from(Array.from({ length: n }, (_, i) => (from + i) % 251))
const sha256 = (buf) => crypto.createHash('sha256').update(buf).digest('hex')
const EXPECTED = sha256(pattern(BYTES))
const bin = (env, name) => process.env[env] ?? path.join(TARGET, 'debug/examples', name)
const once = (emitter, event) => new Promise((resolve) => emitter.once(event, resolve))
const within = (p, ms, what) => Promise.race([p, sleep(ms).then(() => { throw new Error(`timeout waiting for ${what}`) })])

class Ctx {
  constructor(name) {
    this.name = name
    this.procs = []
    this.nodes = []
    this.notes = []
  }

  note(text) {
    this.notes.push(text)
    fs.appendFileSync(path.join(LOG_DIR, `${this.name}-js.log`), `${text}\n`)
  }

  async start() {
    fs.mkdirSync(LOG_DIR, { recursive: true })
    fs.writeFileSync(path.join(LOG_DIR, `${this.name}-js.log`), '')
    const testnet = await startTestnet(4, path.join(LOG_DIR, `${this.name}-testnet.log`))
    this.procs.push(testnet.proc)
    this.bootstrap = testnet.bootstrap
  }

  rust(label, binPath, argv) {
    if (!fs.existsSync(binPath)) throw new Error(`no binary ${binPath} (build it first)`)
    const { cmd, args } = commandFor('rust', [...argv, '--bootstrap', this.bootstrap], binPath)
    const env = { ...process.env, RUST_LOG: process.env.RUST_LOG ?? 'debug' }
    const proc = new Proc(label, cmd, args, { env, logFile: path.join(LOG_DIR, `${this.name}-${label}.log`) })
    this.procs.push(proc)
    return proc
  }

  node(opts) {
    const node = new HyperDHT({ bootstrap: [this.bootstrap], host: '127.0.0.1', ...opts })
    this.nodes.push(node)
    return node
  }

  // JS node as in hyperdht test/relaying.js: ephemeral, no quick firewall classification.
  leaf() { return this.node({ ephemeral: true, quickFirewall: false }) }

  async cleanup() {
    this.onCleanup?.()
    for (const node of this.nodes) await Promise.race([node.destroy({ force: true }).catch(() => {}), sleep(5000)])
    for (const proc of [...this.procs].reverse()) await proc.stop()
  }

  jsTail() { return this.notes.slice(-8).map((n) => `    [js] ${n}`) }

  diag() {
    const rust = this.procs.filter((p) => p.name !== 'testnet').map((p) => p.tail(8)).filter(Boolean)
    return [...rust, ...this.jsTail()].join('\n')
  }
}

// Relay = a plain hyperdht server that hands every connection to blind-relay's accept().
async function startJsRelay(ctx) {
  const node = ctx.node({ ephemeral: false, firewalled: false })
  await node.fullyBootstrapped()
  const counters = { streams: 0 }
  const relay = new RelayServer({
    createStream: (opts) => { counters.streams++; return node.createRawStream({ ...opts, framed: true }) }
  })
  const sockets = []
  const server = node.createServer((socket) => {
    sockets.push(socket)
    ctx.note(`relay: transport socket from ${socket.remotePublicKey.toString('hex').slice(0, 8)}`)
    socket.on('error', (e) => ctx.note(`relay socket error: ${e.message}`))
    const session = relay.accept(socket, { id: socket.remotePublicKey })
    session.on('error', (e) => ctx.note(`relay session error: ${e.message}`))
  })
  await server.listen(keyOf(0x61))
  return { pk: keyOf(0x61).publicKey, sockets, counters }
}

async function startRustRelay(ctx) {
  const proc = ctx.rust('rust-relay', bin('PROBE_BIN_RELAY', 'probe_relay'), ['relay', '--seed', seed(0x62).toString('hex')])
  await proc.waitFor(/LISTENING (\w+)/, 20_000)
  return { pk: keyOf(0x62).publicKey, proc }
}

async function startRustServer(ctx, extra = []) {
  const proc = ctx.rust('rust-server', bin('PROBE_BIN', 'probe'), ['serve', '--seed', seed(0x21).toString('hex'), '--firewalled', ...extra])
  await proc.waitFor(/LISTENING (\w+)/, 20_000)
  return proc
}

async function startJsServer(ctx, opts = {}) {
  let last
  const server = ctx.leaf().createServer({ shareLocalAddress: false, ...opts }, (socket) => {
    last = socket
    socket.on('error', () => {})
    socket.on('open', () => ctx.note(`RAWID server=${socket.rawStream.id}`))
    socket.pipe(socket)
  })
  await server.listen(keyOf(0x11))
  return { rawStream: () => last.rawStream }
}

// Client side: 1 MiB through a half-close, sha256 of what came back.
function echo(socket, onOpen = () => {}) {
  return new Promise((resolve, reject) => {
    const hash = crypto.createHash('sha256')
    let total = 0
    let ended = false
    socket.on('error', reject)
    socket.on('data', (d) => { hash.update(d); total += d.length })
    socket.on('end', () => { ended = true; resolve({ total, digest: hash.digest('hex') }) })
    socket.on('close', () => { if (!ended) reject(new Error('connection closed before the echo ended')) })
    socket.on('open', () => {
      onOpen()
      const data = pattern(BYTES)
      for (let at = 0; at < BYTES; at += CHUNK) socket.write(data.subarray(at, at + CHUNK))
      socket.end()
    })
  })
}

function checkEcho({ total, digest }) {
  if (total !== BYTES) throw new Error(`echo: received ${total} bytes instead of ${BYTES}`)
  if (digest !== EXPECTED) throw new Error(`echo: sha256 ${digest} ≠ ${EXPECTED}`)
}

// JS client → (JS|Rust relay) → (JS|Rust server); the client forbids hole punching.
const relayed = (relayRust, serverRust) => async (ctx) => {
  const relay = relayRust ? await startRustRelay(ctx) : await startJsRelay(ctx)
  const rustServer = serverRust ? await startRustServer(ctx) : null
  if (!serverRust) await startJsServer(ctx)
  const serverPk = keyOf(serverRust ? 0x21 : 0x11).publicKey
  // Without these the JS client reaches the server directly (fastOpen, LAN shortcut, punching) and the relay stays idle.
  const punching = pausePunching(ctx.nodes)
  ctx.onCleanup = punching.restore
  const socket = ctx.leaf().connect(serverPk, {
    keyPair: keyOf(0x12), relayThrough: relay.pk, holepunch: () => false, localConnection: false, fastOpen: false
  })
  socket.on('open', () => ctx.note(`RAWID client=${socket.rawStream.id}`))
  checkEcho(await echo(socket))
  if (rustServer) await rustServer.waitFor(/ECHOED \w+ 1048576/, 5000)
  if (relay.counters && relay.counters.streams < 2) throw new Error('echo ok but the JS relay paired no streams (the connection was direct)')
  if (relay.counters) ctx.note(`JS relay data streams created: ${relay.counters.streams}`)
  if (relay.proc) await relayUsed(ctx, relay.proc)
}

// Rust relay: the STATS line must show a matched pairing, otherwise the echo did not go through it.
async function relayUsed(ctx, proc) {
  const last = () => proc.text.split('\n').filter((l) => l.startsWith('STATS')).pop() ?? 'no STATS'
  await sleep(500)
  ctx.note(`rust relay: ${last()}`)
  if (!/pairings_matched=[1-9]/.test(last())) throw new Error(`echo ok but the Rust relay matched no pairing: ${last()}`)
}

// Holepuncher._punch of the given nodes is held until resume() (as pausePunching in hyperdht test/relaying.js).
// An initiator's own direct path is closed as well: the server's probes (its passive puncher sends one a second,
// about a second after the holepunch round, regardless of its own punch delay) make the JS initiator change the
// stream's remote at once (connect.js c.onsocket), and the probes keep coming, so dropping them until resume() is safe.
function pausePunching(nodes) {
  const punch = Holepuncher.prototype._punch
  const onmessage = Holepuncher.prototype._onholepunchmessage
  const trace = process.env.PROBE_TRACE_PUNCH === '1' // optional evidence: where the packets before resume() come from
  const keep = process.env.PROBE_KEEP_PUNCH_MSGS === '1' // optional: do not drop messages before resume()
  const tr = (text) => { if (trace) console.log(`PUNCHTRACE ${new Date().toISOString()} ${text}`) }
  let resume, paused = true
  const resumed = new Promise((r) => { resume = () => { tr('resume()'); paused = false; r() } })
  Holepuncher.prototype._punch = async function () {
    if (nodes.includes(this.dht)) { tr('_punch held'); await resumed; tr('_punch released') }
    return punch.call(this)
  }
  Holepuncher.prototype._onholepunchmessage = function (...args) {
    if (!nodes.includes(this.dht)) return onmessage.apply(this, args)
    const drop = paused && this.isInitiator && !keep
    const [msg, addr] = args
    if (drop) {
      tr(`initiator=${this.isInitiator} paused=${paused} from=${addr?.host}:${addr?.port} len=${msg?.length} action=dropped connected_after=${this.connected}`)
      return
    }
    const res = onmessage.apply(this, args)
    tr(`initiator=${this.isInitiator} paused=${paused} from=${addr?.host}:${addr?.port} len=${msg?.length} action=processed connected_after=${this.connected}`)
    return res
  }
  return {
    resume,
    restore: () => { resume(); Holepuncher.prototype._punch = punch; Holepuncher.prototype._onholepunchmessage = onmessage }
  }
}

// Upgrade control (JS only): relayed echo of the 1st half, punching resumes, the upgrade is observed, echo of the 2nd half.
// Signal: client rawStream 'remote-changed' + both relay transport sockets 'close' + client remotePort == server localPort.
async function upgradeControl(ctx) {
  const relay = await startJsRelay(ctx)
  const serverNode = ctx.leaf()
  const clientNode = ctx.leaf()
  const punching = pausePunching([serverNode, clientNode])
  try {
    let serverSocket
    const serverOpened = new Promise((resolve) => {
      const server = serverNode.createServer({ relayThrough: relay.pk, shareLocalAddress: false }, (socket) => {
        serverSocket = socket
        socket.on('error', () => {})
        socket.pipe(socket)
        resolve(socket)
      })
      server.listen(keyOf(0x11))
    })
    await sleep(500)
    const client = clientNode.connect(keyOf(0x11).publicKey, { keyPair: keyOf(0x12), fastOpen: false, localConnection: false })
    client.on('error', (e) => ctx.note(`client error: ${e.message}`))
    await within(Promise.all([serverOpened, once(client, 'open')]), 20_000, 'open on both sides')
    const raw = client.rawStream
    if (raw.remotePort === serverSocket.rawStream.localPort) throw new Error('client starts on the direct path, not the relay')
    const hash = crypto.createHash('sha256')
    let total = 0
    const half = BYTES / 2
    const firstHalf = new Promise((resolve) => {
      client.on('data', (d) => { hash.update(d); total += d.length; if (total >= half) resolve() })
    })
    client.write(pattern(half))
    await within(firstHalf, 20_000, 'echo of the 1st half over the relay')
    ctx.note(`before upgrade: echoed ${total} bytes, client remotePort=${raw.remotePort}, server localPort=${serverSocket.rawStream.localPort}`)
    const changed = once(raw, 'remote-changed')
    const relayClosed = Promise.all(relay.sockets.map((s) => (s.destroyed ? null : once(s, 'close'))))
    punching.resume()
    await within(changed, 20_000, "client rawStream 'remote-changed'")
    await within(relayClosed, 20_000, 'relay transport sockets to close')
    if (relay.sockets.length !== 2) throw new Error(`relay saw ${relay.sockets.length} transport sockets instead of 2`)
    if (raw.remotePort !== serverSocket.rawStream.localPort) throw new Error('remotePort did not switch to the server address')
    console.log(`  upgrade observed: remote-changed, ${relay.sockets.length} relay sockets closed, client remotePort == server localPort (${raw.remotePort})`)
    const ended = once(client, 'end')
    client.write(pattern(half, half))
    client.end()
    await within(ended, 20_000, 'echo end after upgrade')
    checkEcho({ total, digest: hash.digest('hex') })
  } finally {
    punching.restore()
  }
}

const SCENARIOS = {
  'js-relay-control': relayed(false, false),
  'js-via-rust-relay': relayed(true, false),
  'js-client-js-relay-rust-server': relayed(false, true),
  'js-client-rust-relay-rust-server': relayed(true, true),
  'js-upgrade-control': upgradeControl
}

Object.assign(SCENARIOS, rsScenarios({
  keyOf, seed, bin, BYTES, EXPECTED, sleep, within, once, pausePunching, startJsRelay, startRustRelay, startRustServer, startJsServer
}))

async function runScenario(name) {
  const ctx = new Ctx(name)
  let timer
  try {
    await ctx.start()
    const limit = new Promise((_, reject) => { timer = setTimeout(() => reject(new Error(`timeout ${SCENARIO_MS} ms`)), SCENARIO_MS) })
    await Promise.race([SCENARIOS[name](ctx), limit])
    console.log(`${name} → ok`)
    return true
  } catch (e) {
    console.log(`${name} → fail: ${e.message}`)
    // waitFor errors already carry the process tail
    console.log(e.message.includes('\n    [') ? ctx.jsTail().join('\n') : ctx.diag())
    return false
  } finally {
    clearTimeout(timer)
    await ctx.cleanup()
  }
}

const wanted = process.argv.slice(2)
const unknown = wanted.filter((n) => !(n in SCENARIOS))
if (unknown.length > 0) {
  console.error(`unknown scenarios: ${unknown.join(', ')}; available: ${Object.keys(SCENARIOS).join(', ')}`)
  process.exit(2)
}
let allOk = true
for (const name of wanted.length > 0 ? wanted : Object.keys(SCENARIOS)) allOk = (await runScenario(name)) && allOk
if (allOk) console.log('ALL OK')
process.exit(allOk ? 0 : 1)
