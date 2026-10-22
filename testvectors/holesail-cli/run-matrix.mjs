// Run of the reference holesail 2.4.4 over a list of cases: arguments -> stdout/stderr/exit code/trace of DHT calls.
// DHT is replaced by a stub (mock-dht.cjs), no network is started. Run: node run-matrix.mjs > cli-matrix.json
import { spawn } from 'node:child_process'
import { mkdtempSync, readFileSync, rmSync, existsSync, writeFileSync, mkdirSync } from 'node:fs'
import { createServer } from 'node:net'
import { tmpdir } from 'node:os'
import { join, resolve, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const root = resolve(here, '../..')
const BIN = join(root, '.tmp/npm-holesail/node_modules/holesail/src/bin/holesail.mjs')
const MOCK = join(here, 'mock-dht.cjs')
const RND = '000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f'
const K64 = 'a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90'
const K40 = 'abcdefghijklmnopqrstuvwxyz0123456789ABCD' // 40 characters
const K31 = 'abcdefghijklmnopqrstuvwxyz01234' // 31
const K32 = 'abcdefghijklmnopqrstuvwxyz012345' // 32
const Z52 = 'ybndrfg8ejkmcpqxot1uwisza345h769ybndrfg8ejkmcpqxot1' // 52 z32 characters, 6th character 'f'
const Z52S = 'ybndrsg8ejkmcpqxot1uwisza345h769ybndrfg8ejkmcpqxot1' // 6th character 's'
const PUB_REC = JSON.stringify({ host: '10.1.2.3', port: 4000, udp: false })
const UDP_REC = JSON.stringify({ host: '0.0.0.0', port: 4100, udp: true })

// [id, args, {env, wait(ms), pre}]
const cases = [
  ['help-noargs', []], ['help-flag', ['--help']], ['help-short', ['-h']], ['help-live', ['--help', 'live']],
  ['help-connect', ['--help', 'connect']], ['help-filemanager', ['--help', 'filemanager']], ['help-unknown', ['--help', 'foo']],
  ['help-short-arg', ['-h', 'live']], ['help-after-invalid', ['--live', 'abc', '--help']],
  ['version', ['--version']], ['version-short', ['-v']], ['version-before-live', ['--version', '--live', '3000']],
  ['unknown-flag', ['--foo']], ['unknown-positional-only-flag-arg', ['--foo', 'bar']],
  ['live-bare', ['--live']], ['live-abc', ['--live', 'abc']], ['live-zero', ['--live', '0']], ['live-neg', ['--live', '-5']],
  ['live-float', ['--live', '3000.5']], ['live-hex', ['--live', '0x10']], ['live-twice', ['--live', '1', '--live', '2']],
  ['live-eq', ['--live=3000']],
  ['port-abc', ['--port', 'abc', '--connect', Z52]], ['port-empty', ['--port=', '--connect', Z52]], ['port-bare', ['--port', '--connect', Z52]],
  ['key-bare', ['--key']], ['key-bare-live', ['--live', '3000', '--key']], ['key-short', ['--live', '3000', '--key', 'abc']],
  ['key-31', ['--live', '3000', '--key', K31]], ['key-short-force', ['--live', '3000', '--key', 'abc', '--force']],
  ['role-bad', ['--filemanager', '.', '--role', 'root']],
  ['connect-two', ['--connect', Z52, Z52S]], ['fm-udp', ['--filemanager', '.', '--udp']],
  ['fm-live', ['--filemanager', '.', '--live', '3000']], ['fm-connect', ['--filemanager', '.', '--connect', Z52]],
  ['fm-connect-pos', ['--filemanager', '.', Z52]], ['fm-nopath', ['--filemanager', '/nonexistent/path']],
  ['fm-bare', ['--filemanager']],
  ['udp-warn-localhost', ['--live', '3000', '--udp', '--host', 'localhost'], { wait: 1500 }],
  ['udp-warn-0000', ['--live', '3000', '--udp', '--host', '0.0.0.0'], { wait: 1500 }],
  // --- server (DHT replaced)
  ['live-private', ['--live', '3000']], ['live-public', ['--live', '3000', '--public']], ['live-public-eq', ['--live', '3000', '--public=false']],
  ['live-udp', ['--live', '3000', '--udp']], ['live-host', ['--live', '3000', '--host', '0.0.0.0']],
  ['live-key40', ['--live', '3000', '--key', K40]], ['live-key32', ['--live', '3000', '--key', K32]],
  ['live-key40-public', ['--live', '3000', '--key', K40, '--public']],
  ['live-key-url-private', ['--live', '3000', '--key', 'hs://s000' + K40]],
  ['live-key-url-private-public', ['--live', '3000', '--key', 'hs://s000' + K40, '--public']],
  ['live-key-url-public', ['--live', '3000', '--key', 'hs://0000' + K40]],
  ['live-key-url-shortkey-force', ['--live', '3000', '--key', 'hs://s000abc', '--force']],
  ['live-key-hex-b', ['--live', '3000', '--key', K64]],
  ['live-port-ignored', ['--live', '3000', '--port', '8080']],
  ['live-log', ['--live', '3000', '--log']], ['live-log0', ['--live', '3000', '--log', '0']], ['live-log9', ['--live', '3000', '--log', '9']],
  ['live-log-abc', ['--live', '3000', '--log', 'abc']], ['live-log-eq2', ['--live', '3000', '--log=2']],
  ['live-fake-conn', ['--live', '3000', '--log', '0'], { env: { MOCK_FAKE_CONN: '1' } }],
  ['live-big-port', ['--live', '99999']],
  // --- client (DHT replaced; the client really listens on a local port)
  ['conn-public-z32', ['--connect', Z52, '--port', '18901']],
  ['conn-pos-public-z32', [Z52, '--port', '18902']],
  ['conn-pos-two', [Z52, Z52S, '--port', '18903']],
  ['conn-z32-6s', ['--connect', Z52S, '--port', '18904']],
  ['conn-url-0000', ['--connect', 'hs://0000' + Z52, '--port', '18905']],
  ['conn-url-s000', ['--connect', 'hs://s000' + K64, '--port', '18906']],
  ['conn-url-s000-public-flag', ['--connect', 'hs://s000' + K64, '--public', '--port', '18907']],
  ['conn-hex-noprefix', ['--connect', K64, '--port', '18908']],
  ['conn-hex-noprefix-public-flag', ['--connect', K64, '--public', '--port', '18909']],
  ['conn-z32-public-flag', ['--connect', Z52, '--public', '--port', '18910']],
  ['conn-invalid-chars', ['--connect', 'not!valid', '--port', '18911']],
  ['conn-default-port', ['--connect', Z52]],
  ['conn-record-host-port', ['--connect', Z52], { env: { MOCK_RECORD_FOR: '*', MOCK_RECORD: PUB_REC } }],
  ['conn-record-udp', ['--connect', Z52], { env: { MOCK_RECORD_FOR: '*', MOCK_RECORD: UDP_REC } }],
  ['conn-record-override', ['--connect', Z52, '--host', '127.0.0.1', '--port', '18912', '--udp'], { env: { MOCK_RECORD_FOR: '*', MOCK_RECORD: PUB_REC } }],
  ['conn-record-badjson', ['--connect', Z52, '--port', '18913'], { env: { MOCK_RECORD_FOR: '*', MOCK_RECORD: 'not json' } }],
  ['conn-udp', ['--connect', Z52, '--udp', '--port', '18914']],
  ['conn-log', ['--connect', Z52, '--port', '18915', '--log', '0']],
  ['conn-port-zero', ['--connect', Z52, '--port', '0']],
  ['conn-port-inuse', ['--connect', Z52, '--port', '18920'], { pre: 'occupy:18920' }],
  ['conn-eq-syntax', ['--connect=' + Z52, '--port=18916']],
  // --- lookup
  ['lookup-none', ['--lookup', Z52]], ['lookup-bare', ['--lookup']], ['lookup-invalid', ['--lookup', 'not!valid']],
  ['lookup-found-public', ['--lookup', Z52], { env: { MOCK_RECORD_FOR: '*', MOCK_RECORD: PUB_REC } }],
  ['lookup-found-udp', ['--lookup', Z52], { env: { MOCK_RECORD_FOR: '*', MOCK_RECORD: UDP_REC } }],
  ['lookup-url-private', ['--lookup', 'hs://s000' + K64], { env: { MOCK_RECORD_FOR: '*', MOCK_RECORD: PUB_REC } }],
  ['lookup-url-public', ['--lookup', 'hs://0000' + Z52], { env: { MOCK_RECORD_FOR: '*', MOCK_RECORD: PUB_REC } }],
  ['lookup-badjson', ['--lookup', Z52], { env: { MOCK_RECORD_FOR: '*', MOCK_RECORD: 'not json' } }],
  ['lookup-empty-obj', ['--lookup', Z52], { env: { MOCK_RECORD_FOR: '*', MOCK_RECORD: '{}' } }],
  // --- filemanager
  ['fm-default', ['--filemanager', '.'], { wait: 2000 }],
  ['fm-opts', ['--filemanager', '.', '--username', 'bob', '--password', 'pw', '--role', 'admin', '--port', '18930', '--host', '127.0.0.1', '--public'], { wait: 2000 }],
  ['fm-key', ['--filemanager', '.', '--port', '18931', '--key', K40], { wait: 2000 }],
  ['fm-port-inuse', ['--filemanager', '.', '--port', '18932'], { pre: 'occupy:18932', wait: 2000 }]
]

// The absolute path of the project root in the output (Node stacks) is machine-dependent: in the JSON it is written as <root>.
const portable = (s) => s.replaceAll(root, '<root>')
const only = process.argv[2]
const out = []
for (const [id, args, o = {}] of cases) {
  if (only && !id.startsWith(only)) continue
  const dir = mkdtempSync(join(tmpdir(), 'hs-matrix-'))
  const trace = join(dir, 'trace.jsonl')
  const occupied = []
  if (o.pre?.startsWith('occupy:')) {
    const s = createServer(); await new Promise((r) => s.listen(+o.pre.slice(7), '127.0.0.1', r)); occupied.push(s)
  }
  const env = { ...process.env, MOCK_TRACE: trace, MOCK_RANDOM_HEX: RND, ...(o.env || {}) }
  const child = spawn('node', ['-r', MOCK, BIN, ...args], { cwd: dir, env, stdio: ['ignore', 'pipe', 'pipe'] })
  let stdout = ''; let stderr = ''; let exited = null; let last = Date.now()
  child.stdout.on('data', (d) => { stdout += d; last = Date.now() })
  child.stderr.on('data', (d) => { stderr += d; last = Date.now() })
  child.on('close', (code, signal) => { exited = { code, signal } })
  const quiet = o.wait || 1200
  const t0 = Date.now()
  while (!exited && (Date.now() - last < quiet) && Date.now() - t0 < 8000) await new Promise((r) => setTimeout(r, 100))
  let running = false; let afterSigint = null
  if (!exited) {
    running = true; child.kill('SIGINT')
    const t1 = Date.now(); while (!exited && Date.now() - t1 < 2500) await new Promise((r) => setTimeout(r, 50))
    if (!exited) { child.kill('SIGKILL'); await new Promise((r) => setTimeout(r, 200)) }
    afterSigint = exited
  }
  for (const s of occupied) s.close()
  const tr = existsSync(trace) ? readFileSync(trace, 'utf8').trim().split('\n').filter(Boolean).map((l) => JSON.parse(l)) : []
  out.push({ id, args, env: o.env || null, running, exit: running ? null : exited, afterSigint, stdout: portable(stdout), stderr: portable(stderr), trace: tr })
  rmSync(dir, { recursive: true, force: true })
  process.stderr.write(`${id}: ${running ? 'running->' + JSON.stringify(afterSigint) : JSON.stringify(exited)}\n`)
}
writeFileSync(join(here, only ? 'cli-matrix.part.json' : 'cli-matrix.json'), JSON.stringify(out, null, 1))
