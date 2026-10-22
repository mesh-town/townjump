// Probe "connect to a key nobody serves": which code the JS hyperdht 6.34.0 returns (`findAndConnect`, `hyperdht/lib/connect.js`).
// Scenarios on a local testnet 127.0.0.1: A — no holders; B — the holder refuses (firewall → true); C — the holder is in the DHT record,
// but is itself stopped (destroy force); D — the holder is alive (control). Observed: A — PEER_NOT_FOUND (~10 ms), B and C —
// PEER_CONNECTION_FAILED (~8 s), D — open (~8 ms). Result: `attempts ? PEER_CONNECTION_FAILED : PEER_NOT_FOUND`.
// Run (reference — interop/setup-js.sh or NM=<path to node_modules>): node testvectors/probes-js/fork-connect-notfound-probe-notfound.cjs
const path = require('path')
const NM = process.env.NM || require('path').join(__dirname, '../../.tmp/npm-holesail/node_modules')
const DHT = require(path.join(NM, 'hyperdht'))
const createTestnet = require(path.join(NM, 'hyperdht/testnet'))

const seed = (n) => Buffer.alloc(32, n)
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))

async function attempt(testnet, kp, label) {
  const client = testnet.createNode()
  const t0 = Date.now()
  let out
  try {
    const sock = client.connect(kp.publicKey)
    await new Promise((res, rej) => {
      sock.once('open', res)
      sock.once('error', rej)
      sock.once('close', () => rej(new Error('closed without open')))
    })
    out = 'open'
    sock.destroy()
  } catch (e) {
    out = `${e.code}: ${e.message}`
  }
  console.log(`${label} -> ${out} (${Date.now() - t0} ms)`)
  await client.destroy({ force: true })
}

async function main() {
  const testnet = await createTestnet(6)
  try {
    await attempt(testnet, DHT.keyPair(seed(1)), 'A no holder')

    const b = testnet.createNode({ ephemeral: false, firewalled: false })
    const kb = DHT.keyPair(seed(2))
    const sb = b.createServer({ firewall: () => true })
    await sb.listen(kb)
    await attempt(testnet, kb, 'B holder refuses (firewall true)')
    await b.destroy({ force: true })

    const c = testnet.createNode({ ephemeral: false, firewalled: false })
    const kc = DHT.keyPair(seed(3))
    const sc = c.createServer()
    await sc.listen(kc)
    await c.destroy({ force: true })
    await attempt(testnet, kc, 'C holder in the record, itself stopped')

    const d = testnet.createNode({ ephemeral: false, firewalled: false })
    const kd = DHT.keyPair(seed(4))
    const sd = d.createServer((s) => s.end())
    await sd.listen(kd)
    await attempt(testnet, kd, 'D holder alive (control)')
    await d.destroy({ force: true })
  } finally {
    await testnet.destroy()
  }
}
main().then(() => process.exit(0), (e) => { console.error(e); process.exit(1) })
