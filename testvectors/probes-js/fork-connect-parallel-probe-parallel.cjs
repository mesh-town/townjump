// Probe "how many holders a connect tries at once": JS hyperdht 6.34.0 (`findAndConnect`, `Semaphore(2)`) on a testnet 127.0.0.1.
// Holders are testnet nodes that store the server record (the server is an ephemeral node, otherwise it answers FIND_PEER itself and becomes a holder);
// "silent" = the node has router.onpeerhandshake replaced (a request without a response), "error" = req.error(...). The log is the starts/finishes of the client peerHandshake.
// Result: 2 attempts in parallel, a silent holder drops off after ~4 s, all silent — 8 s and PEER_CONNECTION_FAILED.
// Run (reference — interop/setup-js.sh or NM=<path to node_modules>): node testvectors/probes-js/fork-connect-parallel-probe-parallel.cjs
const path = require('path')
const NM = process.env.NM || require('path').join(__dirname, '../../.tmp/npm-holesail/node_modules')
const DHT = require(path.join(NM, 'hyperdht'))
const createTestnet = require(path.join(NM, 'hyperdht/testnet'))
const seed = (n) => Buffer.alloc(32, n)

async function holders(testnet, kp) {
  const c = testnet.createNode()
  const out = []
  for await (const d of c.findPeer(kp.publicKey)) {
    if (d.peer) out.push(`${d.from.host}:${d.from.port}`)
  }
  await c.destroy({ force: true })
  return out
}

async function run(label, size, behave) {
  const testnet = await createTestnet(size)
  const b = testnet.createNode({ ephemeral: true })
  const kb = DHT.keyPair(seed(9))
  const sb = b.createServer((s) => { s.on('error', () => {}); s.end() })
  await sb.listen(kb)
  const hs = await holders(testnet, kb).catch((e) => ['err ' + e.message])
  // node rank = the order in which the client PEER_HANDSHAKE first reached it (repeats to the same node do not count)
  const ranks = new Map()
  testnet.nodes.filter((n) => n !== b).forEach((n) => {
    const orig = n._router.onpeerhandshake.bind(n._router)
    n._router.onpeerhandshake = async (req) => {
      const port = n.address().port
      if (!ranks.has(port)) ranks.set(port, ranks.size)
      const how = behave(ranks.get(port), hs.length)
      if (process.env.DBG) console.log('   [node]', port, 'rank', ranks.get(port), how, 'from', req.from.port, 'srv', n === b._dht, n === b)
      if (how === 'silent') return
      if (how === 'error') return req.error(1)
      return orig(req)
    }
  })
  const order = []
  const client = testnet.createNode()
  const t0 = Date.now()
  const log = []
  const orig = client._router.peerHandshake.bind(client._router)
  client._router.peerHandshake = async (target, o, to) => {
    const s = Date.now() - t0
    if (!order.includes(to.port)) order.push(to.port)
    const rank = order.indexOf(to.port)
    try { const r = await orig(target, o, to); log.push(`h${rank} ${to.port} +${s}..${Date.now() - t0} ok`); return r }
    catch (e) { log.push(`h${rank} ${to.port} +${s}..${Date.now() - t0} ${e.code || e.message}`); throw e }
  }
  let out
  try {
    const sock = client.connect(kb.publicKey)
    await new Promise((res, rej) => { sock.on('error', () => {}); sock.once('open', res); sock.once('error', rej); sock.once('close', () => rej(new Error('closed'))) })
    out = 'open'; sock.destroy()
  } catch (e) { out = `${e.code}: ${e.message}` }
  console.log(`${label}: holders=${hs.length} -> ${out} (${Date.now() - t0} ms)`)
  console.log('   ' + log.join('\n   '))
  await new Promise((r) => setTimeout(r, 300))
  await client.destroy({ force: true }); await b.destroy({ force: true }); await testnet.destroy()
}

async function main() {
  const only = process.argv[2]
  const cases = {
    a1: ['a1 two nodes in the testnet, all alive', 2, () => 'ok'],
    aN: ['aN several holders, all alive', 6, () => 'ok'],
    b: ['b the first is silent, the rest alive', 6, (r) => (r === 0 ? 'silent' : 'ok')],
    be: ['be the first answers with an error, the rest alive', 6, (r) => (r === 0 ? 'error' : 'ok')],
    b2: ['b2 the first two are silent, the rest alive', 6, (r) => (r < 2 ? 'silent' : 'ok')],
    c: ['c all silent', 6, () => 'silent'],
    ce: ['ce all answer with an error', 6, () => 'error'],
  }
  for (const k of Object.keys(cases)) if (!only || only === k) await run(...cases[k])
}
main().then(() => process.exit(0), (e) => { console.error(e); process.exit(1) })
