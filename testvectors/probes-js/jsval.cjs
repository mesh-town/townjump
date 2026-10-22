// Marks for jsval: record_tests.rs:49 (`??`) and port_tests.rs:112 (what Node throws on a non-numeric port).
const net = require('net')
const out = { nullish_undefined_null: (undefined ?? null) === null }
const vals = { true: true, false: false, null: null, '[]': [], '{}': {}, undefined: undefined, '"80"': '80', '"abc"': 'abc', '""': '', '-1': -1, '65536': 65536, '1.5': 1.5, NaN: NaN }
const grab = (f) => { try { f(); return 'no throw' } catch (e) { return `${e.name} ${e.code}: ${e.message}` } }
out.connect = {}; out.listen = {}; out.listenObj = {}; out.plusConnect = {}
for (const [k, v] of Object.entries(vals)) {
  out.connect[k] = grab(() => { const s = net.connect({ port: v, host: '127.0.0.1' }); s.on('error', () => {}); s.destroy() })
  out.listen[k] = grab(() => { const s = net.createServer(); s.on('error', () => {}); s.listen(v, '127.0.0.1'); s.close() })
  out.listenObj[k] = grab(() => { const s = net.createServer(); s.on('error', () => {}); s.listen({ port: v, host: '127.0.0.1' }); s.close() })
  out.plusConnect[k] = grab(() => { const s = net.connect({ port: +v, host: '127.0.0.1' }); s.on('error', () => {}); s.destroy() }) + ` (+v=${String(+v)})`
}
console.log(JSON.stringify(out, null, 1))
setTimeout(() => process.exit(0), 200)
