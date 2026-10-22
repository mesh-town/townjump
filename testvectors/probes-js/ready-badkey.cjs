const NM = require('path').join(__dirname, '../../.tmp/npm-holesail/node_modules'); // JS reference holesail 2.4.4: interop/setup-js.sh
// Mark for facade_tests.rs:219: where a non-z32 key throws — in `new Holesail` or in `ready()`.
const Holesail = require(NM + '/holesail')
;(async () => {
  const out = {}
  for (const key of ['not!valid', '!!!']) {
    const lines = []
    let h
    try { h = new Holesail({ client: true, key, port: 0, log: (l) => lines.push(l) }); out[key] = { new: 'ok' } } catch (e) { out[key] = { new: 'threw ' + e.message }; continue }
    try { await h.ready(); out[key].ready = 'resolved' } catch (e) { out[key].ready = { name: e.name, message: e.message } }
    out[key].logLines = lines.length
  }
  console.log(JSON.stringify(out, null, 1)); process.exit(0)
})()
