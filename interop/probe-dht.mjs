// A6b: probe of the peeroxide-dht server and client against the JS hyperdht on a local testnet (127.0.0.1).
// Run: TOWNJUMP_JS_NM=<node_modules of the reference> CARGO_TARGET_DIR=<target> node interop/probe-dht.mjs [scenario names]
// Binary — PROBE_BIN or `cargo build -p peeroxide-dht --example probe` (fork/peeroxide). Each scenario:
// its own testnet of 4 nodes, a 1 MB echo (byte i = i % 251), sha256 of the received equals sha256 of the sent, timeout 45 s.
// Output: `<name> → ok` / `<name> → fail: <reason>`, at the end `ALL OK` (code 0) or code 1.
import crypto from 'node:crypto'
import fs from 'node:fs'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { createRequire } from 'node:module'
import { commandFor, ROOT_DIR } from './lib/impl.mjs'
import { Proc, sleep } from './lib/proc.mjs'
import { startTestnet } from './lib/testnet.mjs'

const require = createRequire(import.meta.url)
const HyperDHT = require('./js/nm.cjs').req('hyperdht')

const BYTES = 1 << 20
const CHUNK = 64 * 1024
const SCENARIO_MS = 45_000
const LOG_DIR = path.join(ROOT_DIR, '.tmp/probe-dht')

const seed = (tag) => Buffer.alloc(32, tag)
const keyOf = (tag) => HyperDHT.keyPair(seed(tag))
const pattern = (n) => Buffer.from(Array.from({ length: n }, (_, i) => i % 251))
const sha256 = (buf) => crypto.createHash('sha256').update(buf).digest('hex')
const EXPECTED = sha256(pattern(BYTES))

function probeBin() {
  if (process.env.PROBE_BIN) return process.env.PROBE_BIN
  const manifest = path.join(ROOT_DIR, 'fork/peeroxide/Cargo.toml')
  const built = spawnSync('cargo', ['build', '--manifest-path', manifest, '-p', 'peeroxide-dht', '--example', 'probe'],
    { cwd: ROOT_DIR, stdio: ['ignore', 'inherit', 'inherit'] })
  if (built.status !== 0) throw new Error('cargo build probe failed')
  const target = process.env.CARGO_TARGET_DIR ?? path.join(ROOT_DIR, 'fork/peeroxide/target')
  return path.join(target, 'debug/examples/probe')
}

// Common scenario context: testnet, child processes and JS nodes are stopped in cleanup() in any outcome.
class Ctx {
  constructor(name, bin) {
    this.name = name
    this.bin = bin
    this.procs = []
    this.nodes = []
  }

  async start() {
    fs.mkdirSync(LOG_DIR, { recursive: true })
    const testnet = await startTestnet(4, path.join(LOG_DIR, `${this.name}-testnet.log`))
    this.procs.push(testnet.proc)
    this.bootstrap = testnet.bootstrap
  }

  rust(label, argv) {
    const { cmd, args } = commandFor('rust', [...argv, '--bootstrap', this.bootstrap], this.bin)
    const proc = new Proc(label, cmd, args, { logFile: path.join(LOG_DIR, `${this.name}-${label}.log`) })
    this.procs.push(proc)
    return proc
  }

  // JS node; open=false — "client behind NAT": remoteAddress() is empty, the client announces UNKNOWN.
  async jsNode({ open = true } = {}) {
    const node = new HyperDHT({ bootstrap: [this.bootstrap], ephemeral: false, firewalled: false, host: '127.0.0.1' })
    this.nodes.push(node)
    await node.fullyBootstrapped()
    if (!open) node.remoteAddress = () => null
    else if (!node.remoteAddress()) throw new Error('JS node is not OPEN: remoteAddress() is empty')
    return node
  }

  async cleanup() {
    for (const node of this.nodes) await Promise.race([node.destroy({ force: true }).catch(() => {}), sleep(5000)])
    for (const proc of [...this.procs].reverse()) await proc.stop()
  }

  diag() {
    return this.procs.filter((p) => p.name !== 'testnet').map((p) => p.tail(8)).filter(Boolean).join('\n')
  }
}

// JS client: connect, a 1 MB echo through a half-close, sha256 of the received.
function jsEcho(socket) {
  return new Promise((resolve, reject) => {
    const hash = crypto.createHash('sha256')
    let total = 0
    let ended = false
    socket.on('error', reject)
    socket.on('data', (d) => { hash.update(d); total += d.length })
    socket.on('end', () => { ended = true; resolve({ total, digest: hash.digest('hex') }) })
    socket.on('close', () => { if (!ended) reject(new Error('connection closed before the echo ended')) })
    socket.on('open', () => {
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

// JS client → Rust server `probe serve`; returns the server process.
async function startRustServer(ctx, tag, extra = []) {
  const server = ctx.rust('rust-server', ['serve', '--seed', seed(tag).toString('hex'), ...extra])
  await server.waitFor(/LISTENING (\w+)/, 20_000)
  return server
}

async function jsConnect(node, serverTag, clientTag) {
  return jsEcho(node.connect(keyOf(serverTag).publicKey, { keyPair: keyOf(clientTag) }))
}

// Scenarios 2–5: JS client (open/unknown) → Rust server (open/firewalled).
const jsToRust = (clientOpen, serverFirewalled) => async (ctx) => {
  const server = await startRustServer(ctx, 0x21, serverFirewalled ? ['--firewalled'] : [])
  const node = await ctx.jsNode({ open: clientOpen })
  checkEcho(await jsConnect(node, 0x21, 0x22))
  await server.waitFor(/ECHOED \w+ 1048576/, 5000)
}

const SCENARIOS = {
  'rust-client-js-server': async (ctx) => {
    const node = await ctx.jsNode()
    const server = node.createServer((socket) => { socket.on('error', () => {}); socket.pipe(socket) })
    await server.listen(keyOf(0x11))
    const client = ctx.rust('rust-client', ['connect', '--server', keyOf(0x11).publicKey.toString('hex'),
      '--seed', seed(0x12).toString('hex')])
    const [, total, digest] = (await client.waitFor(/ECHO (\d+) (\w+)/, 40_000))
    checkEcho({ total: Number(total), digest })
  },
  'js-open-rust-open': jsToRust(true, false),
  'js-unknown-rust-open': jsToRust(false, false),
  'js-open-rust-firewalled': jsToRust(true, true),
  'js-unknown-rust-firewalled': jsToRust(false, true),
  'one-key-two-connects': async (ctx) => {
    const server = await startRustServer(ctx, 0x31)
    const node = await ctx.jsNode()
    checkEcho(await jsConnect(node, 0x31, 0x32))
    checkEcho(await jsConnect(node, 0x31, 0x32))
    await server.waitFor(/ECHOED \w+ 1048576[\s\S]*ECHOED \w+ 1048576/, 5000)
    if (server.count(/INCOMING /) !== 2) throw new Error(`INCOMING: ${server.count(/INCOMING /)} instead of 2`)
  },
  'private-same-key': async (ctx) => {
    const server = await startRustServer(ctx, 0x41, ['--private', keyOf(0x42).publicKey.toString('hex')])
    const node = await ctx.jsNode()
    checkEcho(await jsConnect(node, 0x41, 0x42))
    await server.waitFor(/ECHOED \w+ 1048576/, 5000)
  },
  'private-other-key-refused': async (ctx) => {
    const server = await startRustServer(ctx, 0x51, ['--private', keyOf(0x52).publicKey.toString('hex')])
    const node = await ctx.jsNode()
    const outcome = await jsConnect(node, 0x51, 0x53).then(() => 'connected', () => 'refused')
    if (outcome !== 'refused') throw new Error('client with a foreign key connected')
    await sleep(5000)
    if (server.count(/INCOMING /) !== 0) throw new Error('server got an INCOMING')
  }
}

async function runScenario(name, bin) {
  const ctx = new Ctx(name, bin)
  let timer
  try {
    await ctx.start()
    const limit = new Promise((_, reject) => { timer = setTimeout(() => reject(new Error(`timeout ${SCENARIO_MS} ms`)), SCENARIO_MS) })
    await Promise.race([SCENARIOS[name](ctx), limit])
    console.log(`${name} → ok`)
    return true
  } catch (e) {
    console.log(`${name} → fail: ${e.message}`)
    const diag = e.message.includes('\n    [') ? '' : ctx.diag() // waitFor has already attached the output tail
    if (diag) console.log(diag)
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
const bin = probeBin()
let allOk = true
for (const name of wanted.length > 0 ? wanted : Object.keys(SCENARIOS)) allOk = (await runScenario(name, bin)) && allOk
if (allOk) console.log('ALL OK')
process.exit(allOk ? 0 : 1)
