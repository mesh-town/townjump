const NM = require('path').join(__dirname, '../../.tmp/npm-holesail/node_modules'); // JS reference holesail 2.4.4: interop/setup-js.sh
// Marks: server_tests.rs:183 (empty seed), facade_tests.rs:219 (z32 error in new or ready), dht_tests.rs:22 (new HyperDHT is synchronous).
const HS = require(NM + '/holesail-server')
const HC = require(NM + '/holesail-client')
const z32 = require(NM + '/z32')
const Holesail = require(NM + '/holesail/src/index.js')
const HyperDHT = require(NM + '/hyperdht')
;(async () => {
  const out = {}
  for (const seed of ['', undefined, null, 0]) {
    const s = new HS()
    s.generateKeyPair(seed)
    out['generateKeyPair(' + JSON.stringify(seed) + ') seed bytes'] = s.seed.length
    await s.dht.destroy()
  }
  const keys = { 'not!valid': 'not!valid', empty: '', abc: 'abc', l51: 'y'.repeat(51), l52: 'y'.repeat(52), l53: 'y'.repeat(53), 'upper52': 'Y'.repeat(52), 'bad char at 52': 'y'.repeat(51) + '!' }
  for (const [name, key] of Object.entries(keys)) {
    const r = {}
    try { r.decodeLen = z32.decode(key).length } catch (e) { r.decodeThrow = e.message }
    try { new Holesail({ client: true, key, port: 1, host: '127.0.0.1' }); r.newHolesail = 'no throw' } catch (e) { r.newHolesail = 'THROW ' + e.name + ': ' + e.message }
    try { const c = new HC({ key }); r.newClient = 'no throw'; await c.dht.destroy() } catch (e) { r.newClient = 'THROW ' + e.name + ': ' + e.message }
    out['key ' + name] = r
  }
  const t0 = process.hrtime.bigint()
  const dht = new HyperDHT({ bootstrap: ['127.0.0.1:1'] })
  out.newHyperDHT_ms = Number(process.hrtime.bigint() - t0) / 1e6
  const t1 = process.hrtime.bigint()
  await dht.destroy()
  out.destroy_ms = Number(process.hrtime.bigint() - t1) / 1e6
  console.log(JSON.stringify(out))
  process.exit(0)
})()
