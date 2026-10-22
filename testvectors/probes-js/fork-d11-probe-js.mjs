// D11: which socket/port the JS server (hyperdht 6.34.0) sends the handshake reply from. Local testnet, 127.0.0.1.
// Run: node testvectors/probes-js/fork-d11-probe-js.mjs [scenario-name-substring]  (reference: TOWNJUMP_JS_NM or .tmp/npm-holesail/node_modules; no network needed)
// Output — `fork-d11-probe-js.out.txt` (ports are random, compare by the lines «arrived on» / «sends from»). The table of facts — `docs/spec/deltas.md`, D11.
// Three cases: direct (FROM_CLIENT to the server port), relay (FROM_RELAY via the holder), second (FROM_SECOND_RELAY:
// the client sends relayAddress=R2, the holder H1 forwards R2, R2 — to the server). Server: open | fw-nolan | fw-lan.
// Print: what the server sees (receiving socket, reply socket, replyFromPuncher), what it sends (port → address), what the holder and the client see.
import { createRequire } from 'node:module'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const HERE = path.dirname(fileURLToPath(import.meta.url))
const NM = process.env.TOWNJUMP_JS_NM || path.resolve(HERE, '../../.tmp/npm-holesail/node_modules')
const require = createRequire(path.join(NM, 'x.js'))
const DHT = require('hyperdht')
const createTestnet = require('hyperdht/testnet')
const Router = require('hyperdht/lib/router')
const Server = require('hyperdht/lib/server')
const { handshake } = require('hyperdht/lib/messages')
const c = require('compact-encoding')

const MODES = ['FROM_CLIENT', 'FROM_SERVER', 'FROM_RELAY', 'FROM_SECOND_RELAY', 'REPLY']
const sockets = [] // { name, sock }
let logging = false
const holders = new Map() // target hex -> [{ node, relay }]

const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
const addr = (a) => (a ? `${a.host}:${a.port}` : 'null')
const portOf = (s) => { try { return s.address().port } catch { return 0 } }
const nameOf = (port) => sockets.find((e) => portOf(e.sock) === port)?.name || `?${port}`
const log = (...a) => logging && console.log('   ', ...a)

function track(node, label) {
  node._label = label
  sockets.push({ name: `${label}.server`, sock: node.io.serverSocket })
  sockets.push({ name: `${label}.client`, sock: node.io.clientSocket })
  const create = node.udx.createSocket.bind(node.udx)
  let n = 0
  node.udx.createSocket = (...args) => {
    const sock = create(...args)
    sockets.push({ name: `${label}.pool${n++}`, sock })
    return sock
  }
  for (const s of [node.io.serverSocket, node.io.clientSocket]) wrapSend(s, label)
  const origCreate = node.udx.createSocket
  node.udx.createSocket = (...args) => { const s = origCreate(...args); wrapSend(s, label); return s }
}

function wrapSend(sock, label) {
  const orig = sock.trySend.bind(sock)
  sock.trySend = (buf, port, host, ttl) => {
    log(`${label} sends from ${nameOf(portOf(sock))}(${portOf(sock)}) -> ${host}:${port} len=${buf.length} byte0=${buf[0]}`)
    return orig(buf, port, host, ttl)
  }
}

Router.prototype.onpeerhandshake = ((orig) => function (req) {
  const hs = req.value && c.decode(handshake, req.value)
  if (hs) {
    log(`${this.dht._label || '?'} PEER_HANDSHAKE ${MODES[hs.mode]} from=${addr(req.from)} arrived on ${nameOf(portOf(req.socket))}` +
      ` peerAddress=${addr(hs.peerAddress)} relayAddress=${addr(hs.relayAddress)}`)
  }
  return orig.call(this, req)
})(Router.prototype.onpeerhandshake)

Server.prototype._onpeerhandshake = ((orig) => async function (msg, req) {
  const r = await orig.call(this, msg, req)
  if (r) log(`SERVER reply: arrived on ${nameOf(portOf(req.socket))}, reply.socket=${r.socket ? nameOf(portOf(r.socket)) : 'null (=arrival socket)'}`)
  return r
})(Server.prototype._onpeerhandshake)

Router.prototype.set = ((orig) => function (target, state) {
  const k = target.toString('hex')
  if (state.relay) holders.set(k, [...(holders.get(k) || []), { node: this.dht, relay: state.relay }])
  return orig.call(this, target, state)
})(Router.prototype.set)

async function firewalledNode(bootstrap, label) {
  const node = new DHT({ bootstrap, host: '127.0.0.1', ephemeral: true })
  node._checkIfFirewalled = async () => true
  await node.fullyBootstrapped()
  track(node, label)
  return node
}

async function startServer(testnet, kind, keyPair) {
  const bootstrap = testnet.bootstrap
  const node = kind === 'open'
    ? new DHT({ bootstrap, host: '127.0.0.1', ephemeral: false, firewalled: false })
    : await firewalledNode(bootstrap, 'S')
  if (kind === 'open') { await node.fullyBootstrapped(); track(node, 'S') }
  const server = node.createServer({ shareLocalAddress: kind === 'fw-lan' })
  server.on('connection', (s) => { s.on('error', () => {}); s.end() })
  await server.listen(keyPair)
  await sleep(1500)
  return { node, server }
}

async function runCase(testnet, kind, which, keyPair) {
  holders.clear()
  const { node: S, server } = await startServer(testnet, kind, keyPair)
  const list = [...holders.values()][0] || []
  const hs = list.filter((h, i, a) => a.findIndex((x) => x.node === h.node) === i)
  const C = await firewalledNode(testnet.bootstrap, 'C')
  C.findPeer = () => ({ async *[Symbol.asyncIterator]() {}, destroy() {}, finished: async () => {} })
  const H1 = hs[0]?.node
  const R2 = hs[1]?.node
  const sAddr = { host: '127.0.0.1', port: portOf(S.io.serverSocket) }
  const nodeAddr = (n) => ({ host: '127.0.0.1', port: portOf(n.io.serverSocket) })
  const orig = C._router.peerHandshake.bind(C._router)
  C._router.peerHandshake = async (t, o, to) => {
    const opts = which === 'second' ? { ...o, relayAddress: nodeAddr(R2) } : o
    try {
      const r = await orig(t, opts, to)
      log(`CLIENT got handshake reply: serverAddress=${addr(r.serverAddress)} clientAddress=${addr(r.clientAddress)} relayed=${r.relayed}`)
      return r
    } catch (e) { log(`CLIENT handshake error: ${e.message}`); throw e }
  }
  const via = which === 'direct' ? sAddr : nodeAddr(H1)
  console.log(`== ${kind} / ${which}: S server=${sAddr.port} client=${portOf(S.io.clientSocket)}; holders=${hs.length}; to=${addr(via)}`)
  logging = true
  const sock = C.connect(keyPair.publicKey, { relayAddresses: [via], localConnection: kind === 'fw-lan' })
  sock.on('error', (e) => log(`connect error: ${e.message}`))
  const opened = await Promise.race([new Promise((r) => sock.once('open', () => r(true))), sleep(9000).then(() => false)])
  await sleep(300)
  logging = false
  console.log(`   connect opened=${opened}`)
  sock.destroy()
  await server.close().catch(() => {})
  await C.destroy(); await S.destroy()
}

const only = process.argv[2]
const testnet = await createTestnet(8)
sockets.push(...testnet.nodes.flatMap((n, i) => {
  n._label = `N${i}`
  return [{ name: `N${i}.server`, sock: n.io.serverSocket }, { name: `N${i}.client`, sock: n.io.clientSocket }]
}))
for (const kind of ['open', 'fw-nolan', 'fw-lan']) {
  for (const which of ['direct', 'relay', 'second']) {
    if (only && !`${kind}/${which}`.includes(only)) continue
    await runCase(testnet, kind, which, DHT.keyPair())
  }
}
await testnet.destroy()
process.exit(0)
