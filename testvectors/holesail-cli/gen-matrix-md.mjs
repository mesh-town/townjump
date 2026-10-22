// Builds docs/spec/holesail/cli-matrix.md from cli-matrix.json (ANSI, frame and QR cut out; exact bytes are in the JSON).
import { readFileSync, writeFileSync } from 'node:fs'
const d = JSON.parse(readFileSync(new URL('./cli-matrix.json', import.meta.url)))
const strip = (s) => s.replace(/\x1b\[[0-9;]*m/g, '')
const esc = (s) => s.replace(/\|/g, '\\|').replace(/`/g, "'")
const short = (s) => s.replace(/\b([0-9a-f]{8})[0-9a-f]{40,}\b/g, '$1…')
function out(c) {
  const raw = strip(c.stdout)
  if (raw.startsWith('Holesail CLI')) return raw.includes('Commands:') ? 'general help' : raw.includes('Reverse proxy') ? 'live help' : raw.includes('Connect to a Holesail') ? 'connect help' : 'filemanager help'
  const lines = raw.split('\n')
  const hasBox = lines.some((l) => l.startsWith('┌'))
  const rows = lines.map((l) => l.replace(/^[|│]/, '').replace(/[│|]\s*$/, '').replace(/\s+/g, ' ').trim()).filter((l) => l && !/^[─┌┐└┘]+$/.test(l) && !/^[ ▀▄█]+$/.test(l))
  const qr = lines.some((l) => /^[ ▀▄█]{20,}$/.test(l))
  const r = rows.map((l) => l.replace(/^\d{4}-\d\d-\d\dT[\d:.]+Z/, '<ts>').replace(/^(NOTE|NOTICE):.*/, '$1:…'))
  return (hasBox ? 'BANNER: ' : '') + r.slice(0, 9).join(' ⏎ ') + (r.length > 9 ? ' ⏎ …' : '') + (qr ? ' ⏎ [QR]' : '')
}
function trace(c) {
  const t = []
  for (const e of c.trace) {
    if (e.ev === 'randomBytes' || e.ev === 'firewall.probe') continue
    if (e.ev === 'HyperDHT.new') t.push('new HyperDHT(' + (e.opts.length ? '{' + e.opts.join(',') + '}' : '') + ')')
    else if (e.ev === 'HyperDHT.keyPair') t.push(`keyPair(seed ${e.seed.slice(0, 8)}…)→pk ${e.publicKey.slice(0, 8)}…`)
    else if (e.ev === 'createServer') t.push(`createServer{firewall:${e.firewall},reusableSocket:${e.reusableSocket}}`)
    else if (e.ev === 'server.listen') t.push(`listen(pk ${e.publicKey.slice(0, 8)}…)`)
    else if (e.ev === 'mutableGet') t.push(`mutableGet(pk ${e.publicKey.slice(0, 8)}…, ${JSON.stringify(e.opts)})`)
    else if (e.ev === 'mutablePut') t.push(`mutablePut(${e.value}, opts ${JSON.stringify(e.optKeys)})`)
    else t.push(e.ev)
  }
  return t.join('; ')
}
const groups = [['help', 'Help and version'], ['version', 'Help and version'], ['unknown', 'Unknown flags'], ['live-', 'Server (--live)'], ['udp-', 'Server (--live)'], ['key-', 'Input validation (validateInput)'], ['port-', 'Input validation (validateInput)'], ['role', 'Input validation (validateInput)'], ['connect-two', 'Input validation (validateInput)'], ['fm-', 'Filemanager'], ['conn-', 'Client (--connect / positional key)'], ['lookup', 'Lookup (--lookup)']]
const gname = (id) => groups.find(([p]) => id.startsWith(p))?.[1] ?? 'Other'
const order = [...new Set(groups.map((g) => g[1]))]
let md = `# Behavior matrix of the holesail 2.4.4 CLI: arguments, output, exit code, side effect

Captured by running the real \`holesail\` 2.4.4 (\`node src/bin/holesail.mjs\`) with the network replaced: the preloader \`node -r testvectors/holesail-cli/mock-dht.cjs\` swaps \`hyperdht\` for a stub that writes calls to a trace and opens no sockets. So the "side effect" is the exact calls of the DHT layer (which arguments \`createServer\`, \`mutablePut\`, \`mutableGet\` received), not network activity. The random 32 bytes are replaced with \`000102…1f\`, hence the identical keys in different rows. How to read:

- Output is stdout only, ANSI colors, the banner frame and the QR are cut out; \`⏎\` is a line feed; \`<ts>\` is an ISO timestamp; hashes are shortened to 8 characters and \`…\`. The exact bytes (colors, frame, QR) are in \`testvectors/holesail-cli/cli-matrix.json\`, field \`stdout\`.
- "Code": a number — the process exited by itself; "runs" — the process stayed alive (the banner is printed), on SIGINT it dies from the signal (the JS version has no handler, shell exit code 130); for the filemanager \`graceful-goodbye\` catches SIGINT and exits with code 130 after \`dht.destroy\`.
- The rows \`live-fake-conn\`, \`conn-record-*\`, \`lookup-found-*\`, \`lookup-badjson\`, \`lookup-empty-obj\` run with a DHT record substituted (\`MOCK_RECORD\`, the "condition" field).
- The network parts (real listen, real connect, data transfer) are not verified in the matrix — for the results of the end-to-end probe see the section about the end-to-end probe in [README.md](README.md). The matrix is suitable for CLI parity tests: output, codes, DHT calls.

`
const cur = new Set()
const secs = {}
for (const c of d) (secs[gname(c.id)] ??= []).push(c)
for (const g of order) {
  md += `\n## ${g}\n\n| id | arguments | condition | code | stdout | stderr | DHT calls |\n|---|---|---|---|---|---|---|\n`
  for (const c of secs[g] ?? []) {
    const code = c.running ? `runs; SIGINT → ${c.afterSigint.signal ? 'signal ' + c.afterSigint.signal : 'code ' + c.afterSigint.code}` : `${c.exit.code}`
    const args = c.args.map((a) => (/[\s"]/.test(a) ? JSON.stringify(a) : a || '""')).join(' ')
    const cond = c.env ? esc(short(Object.entries(c.env).map(([k, v]) => `${k}=${v}`).join(' '))) : ''
    md += `| ${c.id} | \`${esc(short(args))}\` | ${cond} | ${code} | ${esc(short(out(c)))} | ${esc(short(strip(c.stderr).trim().replace(/\n/g, ' ⏎ ').replace(/\d{4}-\d\d-\d\dT[\d:.]+Z/g, '<ts>')))} | ${esc(short(trace(c)))} |\n`
  }
}
writeFileSync(new URL('../../docs/spec/holesail/cli-matrix.md', import.meta.url), md)
console.log(md.split('\n').length, 'lines', md.length, 'chars')
