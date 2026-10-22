// Compact view of cli-matrix.json: node show.mjs [id prefix] — without ANSI, frame and QR.
import { readFileSync } from 'node:fs'
const d = JSON.parse(readFileSync(new URL('./cli-matrix.json', import.meta.url)))
const strip = (s) => s.replace(/\x1b\[[0-9;]*m/g, '')
const p = process.argv[2]
for (const c of d) {
  if (p && !c.id.startsWith(p)) continue
  const vis = strip(c.stdout).split('\n').map((l) => l.replace(/^[|│]/, '').replace(/[│|]\s*$/, '').replace(/\s+/g, ' ').trim()).filter((l) => l && !/^[─┌┐└┘\-+]+$/.test(l) && !/[▀▄█]{3}/.test(l))
  console.log(`### ${c.id}  args=${JSON.stringify(c.args)}  exit=${JSON.stringify(c.exit)} running=${c.running} afterSigint=${JSON.stringify(c.afterSigint)}`)
  if (vis.length) console.log('  out: ' + vis.slice(0, 14).join('\n       '))
  if (c.stderr.trim()) console.log('  err: ' + strip(c.stderr).trim().split('\n').slice(0, 3).join('\n       '))
  if (c.trace.length) console.log('  trace: ' + c.trace.filter((t) => !['randomBytes','HyperDHT.new'].includes(t.ev)).map((t) => JSON.stringify(t).replace(/"([0-9a-f]{8})[0-9a-f]{56}"/g, '"$1.."').replace(/\\"/g, "'")).join('\n         '))
}
