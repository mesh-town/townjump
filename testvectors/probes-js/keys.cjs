const NM = require('path').join(__dirname, '../../.tmp/npm-holesail/node_modules'); // JS reference holesail 2.4.4: interop/setup-js.sh
// Mark for server_keys_tests.rs:228: what the library does with secure: 'false' (a string). No network: the constructor is synchronous.
const Holesail = require(NM + '/holesail/src/index.js')
const show = (o) => { const h = new Holesail(o); return { secure: h.secure, key: h.key, seed: h.seed } }
const out = {}
for (const secure of [true, false, 'false', 'true', 0, 1, undefined, '']) {
  out['server secure=' + JSON.stringify(secure) + ' no key'] = (() => { const r = show({ server: true, port: 1, host: 'h', secure }); return { secure: r.secure, keyIsRandom: !!r.key, seedLen: r.seed && r.seed.length } })()
  out['server secure=' + JSON.stringify(secure) + ' key=K'] = show({ server: true, port: 1, host: 'h', secure, key: 'K'.repeat(32) })
}
// server module: secure === true strictly
const HS = require(NM + '/holesail-server')
out.serverModule = {}
for (const secure of [true, 'false', 1, 'true']) { const s = new HS(); s.secure = secure === true; out.serverModule[JSON.stringify(secure)] = s.secure }
console.log(JSON.stringify(out, null, 1))
setTimeout(() => process.exit(0), 100)
