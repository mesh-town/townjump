const NM = require('path').join(__dirname, '../../.tmp/npm-holesail/node_modules'); // JS reference holesail 2.4.4: interop/setup-js.sh
(async () => {
// Marks for client: client_tests.rs:82 (info before connect), ping_tests.rs:132 (record "null"), the format of "Retrieved DHT data".
const HC = require(NM + '/holesail-client')
const z32 = require(NM + '/z32')
const HyperDHT = require(NM + '/hyperdht')
const out = {}
const pk = Buffer.alloc(32, 7)
const c = new HC({ key: z32.encode(pk) })
try { c.info; out.infoBeforeConnect = 'no throw' } catch (e) { out.infoBeforeConnect = e.name + ': ' + e.message }
out.stateBeforeConnect = c.state
c.dht.destroy()
// ping with a fake dht: mutableGet returns the record by key
async function ping (secureValue, insecureValue) {
  const key = z32.encode(pk)
  const secPk = HyperDHT.keyPair(pk).publicKey
  const dht = { mutableGet: async (p) => { const v = Buffer.compare(p, secPk) === 0 ? secureValue : insecureValue; return v === undefined ? null : { seq: 1, value: Buffer.from(v) } } }
  try { const r = await HC.ping(key, dht); return { type: typeof r, json: JSON.stringify(r), isNull: r === null } } catch (e) { return 'THROW ' + e.name + ': ' + e.message }
}
out.ping = {}
const good = '{"port":1}'
for (const v of ['null', '5', '0', 'true', 'false', '"str"', '""', '[]', '[1]', '{}', '{"udp":true,"port":2}', 'not json']) {
  out.ping['secureRec=' + v + ' + insecureRec=' + good] = await ping(v, good)
  out.ping['secureRec=' + v + ' + no insecure'] = await ping(v, undefined)
}
// Retrieved DHT data = JSON.stringify(JSON.parse(value))
out.retrieved = {}
for (const v of ['{"host":"127.0.0.1", "port": 1e2}', '{ "port" : 8080 , "host" : "a" }', '{"b":1,"a":2,"1":3,"0":4}', '{"port":1.50,"x":-0,"y":1E+30,"z":0.000001}', '{"s":"\u00e9\ud83d\ude00\n\u0007"}', '{"a":1,"a":2}', '[1, 2]', '5', '"str"', '{"udp":true,"host":null}', '{"port":12345678901234567890}']) {
  try { out.retrieved[v] = JSON.stringify(JSON.parse(v)) } catch (e) { out.retrieved[v] = 'THROW ' + e.message }
}
console.log(JSON.stringify(out, null, 1))
process.exit(0)
})()
