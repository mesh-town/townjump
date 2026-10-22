// Speed probe of the reference: JS udx-native on loopback (two sockets 127.0.0.1, one stream, write + drain). What it proves: JS gets 100–140 MiB/s
// (8 MiB in 16 KiB chunks in 0.06–0.09 s; 50 MiB — 0.4 s; the chunk size 1–64 KiB barely matters) — the reference for the fork measurements (`libudx`, `SecretStream`).
// Run (reference — interop/setup-js.sh): node testvectors/probes-js/fork-throughput-probe-udx.cjs [MiB=8] [chunk=16384]; PROBE_TMO_MS — timeout, ms.
// Prints one line `PROBE udx-native bytes=… secs=… mibs=… chunk=…`.
const UDX = require(require('path').join(process.env.TOWNJUMP_JS_NM || require('path').join(__dirname, '../../.tmp/npm-holesail/node_modules'), 'udx-native'))

const mib = Number(process.argv[2] || 8)
const chunk = Number(process.argv[3] || 16384)
const total = mib * 1024 * 1024

const u = new UDX()
const sa = u.createSocket()
const sb = u.createSocket()
sa.bind(0, '127.0.0.1')
sb.bind(0, '127.0.0.1')

const a = u.createStream(1)
const b = u.createStream(2)
a.connect(sa, b.id, sb.address().port, '127.0.0.1')
b.connect(sb, a.id, sa.address().port, '127.0.0.1')

const payload = Buffer.alloc(chunk, 0xa5)
let sent = 0
let received = 0
let start = 0

function pump () {
  while (sent < total) {
    const n = Math.min(chunk, total - sent)
    const ok = a.write(n === chunk ? payload : payload.subarray(0, n))
    sent += n
    if (!ok) { a.once('drain', pump); return }
  }
}

b.on('data', (d) => {
  received += d.length
  if (received >= total) {
    const secs = Number(process.hrtime.bigint() - start) / 1e9
    console.log(`PROBE udx-native bytes=${received} secs=${secs.toFixed(3)} mibs=${(received / 1048576 / secs).toFixed(2)} chunk=${chunk}`)
    a.destroy(); b.destroy()
    sa.close(); sb.close()
  }
})
for (const s of [a, b]) s.on('error', (e) => { console.error('stream error', e.message) })

start = process.hrtime.bigint()
pump()
setTimeout(() => { console.error('TIMEOUT received=' + received); process.exit(2) }, Number(process.env.PROBE_TMO_MS || 120000)).unref()
