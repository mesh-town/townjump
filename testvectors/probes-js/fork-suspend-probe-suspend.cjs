// Probe F9 (`suspend`/`resume` of the fork DHT): what JS `dht.suspend()`/`dht.resume()` (hyperdht 6.34.0) does on a local testnet.
// Shows: suspend breaks established connections (the remote peer gets ECONNRESET in ~3 ms), the port after resume is the same,
// the announce is removed and returned, a suspended node's own connect gives SUSPENDED, repeated suspend/resume are idempotent.
// Prints observations as lines `PROBE <what>: <value>`; checks nothing. Figures as observed.
// Run (needs the reference from interop/setup-js.sh, no network, ~20 s): node testvectors/probes-js/fork-suspend-probe-suspend.cjs
const { req } = require('../../interop/js/nm.cjs')
const HyperDHT = req('hyperdht')
const createTestnet = req('hyperdht/testnet')

const log = (k, v) => console.log(`PROBE ${k}: ${v}`)
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
const attempt = async (p) => p.then(() => 'ok', (e) => `${e.code || e.name}: ${e.message}`)
const t0 = () => Date.now()

async function main() {
  const testnet = await createTestnet(3)
  const bootstrap = testnet.bootstrap
  const opts = { bootstrap, host: '127.0.0.1', ephemeral: false, firewalled: false }
  const serverDht = new HyperDHT(opts)
  const clientDht = new HyperDHT(opts)
  let tSusp = 0
  const kp = HyperDHT.keyPair(Buffer.alloc(32, 7))

  let serverStream = null
  const server = serverDht.createServer((s) => {
    serverStream = s
    s.on('data', (d) => s.write(d))
    s.on('error', () => {})
    s.on('close', () => log('server-side stream close', `t+${Date.now() - tSusp}ms`))
  })
  await server.listen(kp)
  const portBefore = serverDht.address().port
  log('server port before suspend', portBefore)

  const c1 = clientDht.connect(kp.publicKey)
  c1.on('error', (e) => log('client stream error', e.code || e.message))
  c1.on('close', () => log('client stream close', `t+${Date.now() - tSusp}ms`))
  await new Promise((r) => c1.once('open', r))
  c1.write(Buffer.from('ping'))
  await new Promise((r) => c1.once('data', r))
  log('established echo', 'ok')

  tSusp = t0()
  await serverDht.suspend()
  log('suspend took ms', Date.now() - tSusp)
  log('serverDht.suspended', serverDht.suspended)
  log('serverDht.io.serverSocket after suspend', serverDht.io.serverSocket ? 'present' : 'null')
  await sleep(1500)
  log('server stream destroyed', serverStream ? serverStream.destroyed : 'no stream')
  log('client stream destroyed', c1.destroyed)

  const tc = t0()
  log('new client connect while suspended', await attempt(new Promise((res, rej) => {
    const s = clientDht.connect(kp.publicKey)
    s.on('open', () => { s.destroy(); res() })
    s.on('error', rej)
  })))
  log('  connect took ms', Date.now() - tc)

  const tk = t0()
  log('suspended node own connect', await attempt(new Promise((res, rej) => {
    const s = serverDht.connect(HyperDHT.keyPair(Buffer.alloc(32, 9)).publicKey)
    s.on('open', res)
    s.on('error', rej)
  })))
  log('  own connect took ms', Date.now() - tk)

  const tp = t0()
  log('ping suspended node', await attempt(clientDht.ping({ host: '127.0.0.1', port: portBefore })))
  log('  ping took ms', Date.now() - tp)
  log('findPeer during suspend hits', await (async () => {
    let n = 0
    for await (const _ of clientDht.findPeer(kp.publicKey)) n++
    return n
  })())

  await serverDht.resume()
  log('resume done; suspended', serverDht.suspended)
  log('server port after resume', serverDht.address().port)
  await sleep(1500)
  log('ping after resume', await attempt(clientDht.ping({ host: '127.0.0.1', port: portBefore })))
  const s2 = clientDht.connect(kp.publicKey)
  s2.on('error', () => {})
  log('connect after resume', await attempt(new Promise((res, rej) => { s2.on('open', res); s2.on('close', () => rej(new Error('closed'))) })))
  log('double suspend/resume', await attempt((async () => {
    await serverDht.suspend(); await serverDht.suspend(); await serverDht.resume(); await serverDht.resume()
  })()))

  await Promise.allSettled([clientDht.destroy(), serverDht.destroy(), testnet.destroy()])
  process.exit(0)
}
main().catch((e) => { console.error(e); process.exit(1) })
