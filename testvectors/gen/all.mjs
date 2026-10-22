// Regeneration of all vectors: node testvectors/gen/all.mjs. The scripts run one after another, each in its own process; an error in any aborts the run.
// Output — testvectors/<layer>/*.json, without randomness and time (timings are rounded to 100 ms).
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { dirname, join } from 'node:path'
const dir = dirname(fileURLToPath(import.meta.url))
const scripts = ['holesail-keys.mjs', 'holesail-records.mjs', 'wire-codecs.mjs', 'deltas-codec.mjs', 'secret-stream.mjs', 'deltas-plugin.mjs', 'deltas-udx.mjs', 'deltas-udx-probes.mjs','deltas-blind-relay.mjs', 'deltas-handshake-socket.mjs']
for (const s of scripts) {
  const r = spawnSync(process.execPath, [join(dir, s)], { stdio: ['ignore', 'pipe', 'inherit'], encoding: 'utf8' })
  process.stdout.write(`${r.status === 0 ? 'ok  ' : 'FAIL'} ${s}\n`)
  if (r.status !== 0) process.exit(1)
}
