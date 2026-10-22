// Compatibility probe holesail 3.0 (branch 3.0, holesail-client/server 3.0.0-dev.0) ↔ 2.4.4 on a local testnet.
// Run: node interop/holesail3-compat/compat.mjs   (process logs and the result.txt total — .tmp/holesail3-compat-out/)
// All processes are children of this script and are stopped in finally (by PID, not by name).
import { spawn } from 'node:child_process'
import { createRequire } from 'node:module'
import crypto from 'node:crypto'
import fs from 'node:fs'
import net from 'node:net'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { startEcho } from '../js/echo.mjs'
import { parseBannerUrl } from '../lib/banner.mjs'

const HERE = path.dirname(fileURLToPath(import.meta.url))
const ROOT = path.resolve(HERE, '../..')
const LOGS = path.join(ROOT, '.tmp/holesail3-compat-out')
fs.rmSync(LOGS, { recursive: true, force: true })
fs.mkdirSync(LOGS, { recursive: true })
const NODE = process.execPath
const CLI244 = path.join(ROOT, '.tmp/npm-holesail/node_modules/holesail/src/bin/holesail.mjs')
const CLI3 = path.join(ROOT, '.tmp/holesail-3.0/src/bin/holesail.mjs')
const PRE244 = path.join(ROOT, 'interop/js/bootstrap-env.cjs')
const PRE3 = path.join(HERE, 'preload3.cjs')
const strip = (s) => s.replace(/\x1b\[[0-9;]*m/g, '')
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
const procs = []
const results = []
const say = (s) => { console.log(s); results.push(s) }

class Proc {
  constructor(name, args, env) {
    this.name = name
    this.text = ''
    this.log = path.join(LOGS, `${name}.log`)
    this.child = spawn(NODE, args, { cwd: ROOT, env: { ...process.env, ...env }, stdio: ['pipe', 'pipe', 'pipe'] })
    this.exited = null
    for (const s of [this.child.stdout, this.child.stderr]) s.on('data', (d) => { this.text += strip(d.toString()); fs.writeFileSync(this.log, this.text) })
    this.child.on('exit', (code, sig) => { this.exited = `exit code=${code} signal=${sig}`; fs.writeFileSync(this.log, this.text + `\n[${this.exited}]\n`) })
    fs.writeFileSync(this.log, '')
    procs.push(this)
  }
  async waitFor(re, ms) {
    const end = Date.now() + ms
    while (Date.now() < end) {
      if (re.test(this.text)) return true
      if (this.exited) return re.test(this.text)
      await sleep(100)
    }
    return re.test(this.text)
  }
  kill() { if (!this.exited) this.child.kill('SIGTERM') }
  tail(n = 6) { return this.text.trim().split('\n').slice(-n).join('\n') }
}

const js244 = (name, argv, env) => new Proc(name, ['-r', PRE244, CLI244, ...argv], env)
const js3 = (name, argv, env) => new Proc(name, ['-r', PRE3, CLI3, ...argv], env)

// Sends payload to the client's local port, waits for the returned bytes (echo); 700 ms after payload.length is reached — for the tail.
function roundtrip(port, payload, ms = 8000) {
  return new Promise((resolve) => {
    const got = []
    let n = 0
    let err = null
    let closed = false
    let grace = null
    const t0 = Date.now()
    const s = net.connect(port, '127.0.0.1')
    const done = (why) => { clearTimeout(timer); clearTimeout(grace); s.destroy(); resolve({ got: Buffer.concat(got), closed, err, why, ms: Date.now() - t0 }) }
    const timer = setTimeout(() => done('timeout'), ms)
    s.on('connect', () => s.write(payload))
    s.on('data', (d) => { got.push(d); n += d.length; if (n >= payload.length && !grace) grace = setTimeout(() => done('enough'), 700) })
    s.on('error', (e) => { err = e.code || e.message; done('error') })
    s.on('close', () => { closed = true; done('close') })
  })
}

function describe(payload, r) {
  const eq = r.got.equals(payload)
  const pre = r.got.length === payload.length + 33 && r.got.subarray(33).equals(payload)
  const tag = eq ? 'ok, bytes equal' : pre ? 'PARTIAL, the echo returned the payload with 33 extra bytes at the start' : 'FAIL'
  return `${tag} (sent ${payload.length}, returned ${r.got.length}, end: ${r.why}${r.err ? ' ' + r.err : ''}, ${r.ms} ms)`
}

const nm3 = createRequire(path.join(ROOT, '.tmp/holesail-3.0/package.json'))
const z32 = nm3('z32')
const inviteLib = nm3('@holesail/invite')
const rnd = (n) => crypto.randomBytes(n)
const free = () => new Promise((r) => { const s = net.createServer().listen(0, '127.0.0.1', () => { const p = s.address().port; s.close(() => r(p)) }) })
const inviteRe = /hs_[a-z0-9]{100,}/
const errLine = (t) => (t.match(/^(?:Bail: )?\w*Error[^\n]*/m) || [''])[0]

async function cliClient(name, start, url, tail, env) {
  const port = await free()
  const t0 = Date.now()
  const cli = start(name, [...url, '--port', String(port), '--log', '1'], env)
  await cli.waitFor(/Proxy listening|Error/, 20000)
  const ms = Date.now() - t0
  await sleep(1500)
  return { cli, port, ms, listening: /Proxy listening/.test(cli.text) }
}

async function main() {
  const tn = new Proc('testnet', [path.join(ROOT, 'interop/js/testnet.mjs'), '4', '--exit-on-stdin-eof'], {})
  if (!(await tn.waitFor(/TOWNJUMP_BOOTSTRAP=/, 20000))) throw new Error('testnet did not start: ' + tn.text)
  const BOOT = tn.text.match(/TOWNJUMP_BOOTSTRAP=(\S+)/)[1]
  const env = { TOWNJUMP_BOOTSTRAP: BOOT }
  say(`testnet ${BOOT}`)
  const echo = await startEcho()
  const E = echo.tcpPort

  // S0: control 3.0 <- 3.0
  {
    const srv = js3('S0-server3', ['share', String(E), '--log', '0'], env)
    await srv.waitFor(inviteRe, 20000)
    const invite = srv.text.match(inviteRe)[0]
    const c = await cliClient('S0-client3', js3, ['connect', invite], null, env)
    const p = rnd(1000)
    say(`S0 control, server 3.0 <- client 3.0 (CLI, with probe): client ${c.listening ? 'started' : 'NOT started'}; ${describe(p, await roundtrip(c.port, p))}`)
    c.cli.kill(); srv.kill(); await sleep(500)
  }
  // S0b: control 2.4.4 <- 2.4.4
  {
    const srv = js244('S0b-server244', ['--live', String(E), '--log', '0'], env)
    await srv.waitFor(/hs:\/\/(s000|0000)[^\s│|]+/, 20000)
    const url = parseBannerUrl(srv.text).url
    const c = await cliClient('S0b-client244', js244, [url], null, env)
    const p = rnd(1000)
    say(`S0b control, server 2.4.4 (${url.slice(0, 12)}…) <- client 2.4.4: client ${c.listening ? 'started' : 'NOT started'}; ${describe(p, await roundtrip(c.port, p))}`)
    c.cli.kill(); srv.kill(); await sleep(500)
  }

  // (a) server 3.0 <- client 2.4.4
  {
    const srv = js3('A-server3', ['share', String(E), '--log', '0'], env)
    await srv.waitFor(inviteRe, 20000)
    const invite = srv.text.match(inviteRe)[0]
    const c1 = await cliClient('A1-client244-invite', js244, [invite], null, env)
    say(`A1 (a) connection string of server 3.0 «${invite.slice(0, 12)}…» into client 2.4.4: client ${c1.listening ? 'started' : 'NOT started'}; ${errLine(c1.cli.text)}`)
    c1.cli.kill()
    const pub = z32.encode(inviteLib.parse(invite).publicKey)
    const c2 = await cliClient('A2-client244-pubkey', js244, [`hs://0000${pub}`], null, env)
    say(`A2 (a, wire level) public key of server 3.0 as hs://0000<z32> into client 2.4.4: client ${c2.listening ? 'started' : 'NOT started'}`)
    if (c2.listening) {
      const p5 = Buffer.from('ping\n')
      say(`A2 short payload 5 B: ${describe(p5, await roundtrip(c2.port, p5, 6000))}`)
      const p64 = rnd(64)
      say(`A2 payload 64 B: ${describe(p64, await roundtrip(c2.port, p64, 6000))}`)
    }
    await sleep(500)
    say(`A2 log of server 3.0 (--log 0): ${srv.text.split('\n').filter((l) => /Verification|Tunnel|Probe|Unknown|Incoming/.test(l)).join(' | ') || '(no connection lines)'}`)
    c2.cli.kill(); srv.kill(); await sleep(500)
  }

  // (b) server 2.4.4 <- client 3.0
  for (const mode of ['public', 'private']) {
    const argv = ['--live', String(E), '--log', '0']
    if (mode === 'public') argv.push('--public')
    else argv.push('--key', `compat-${rnd(16).toString('hex')}`)
    const srv = js244(`B-server244-${mode}`, argv, env)
    await srv.waitFor(/hs:\/\/(s000|0000)[^\s│|]+/, 20000)
    const url = parseBannerUrl(srv.text).url
    const c1 = await cliClient(`B1-client3-${mode}-url`, js3, ['connect', url], null, env)
    say(`B1 (b, ${mode}) connection string of server 2.4.4 «${url.slice(0, 12)}…» into client 3.0: client ${c1.listening ? 'started' : 'NOT started'}; ${errLine(c1.cli.text)}`)
    c1.cli.kill()
    if (mode === 'public') {
      const key = url.slice('hs://0000'.length)
      const base = Buffer.concat([Buffer.from([1]), Buffer.from(z32.decode(key)), rnd(32)])
      const forged = 'hs_' + z32.encode(Buffer.concat([base, nm3('hypercore-crypto').hash(base).subarray(0, 4)]))
      const c2 = await cliClient('B2-client3-forged-cli', js3, ['connect', forged], null, env)
      say(`B2 (b, wire level) 3.0 invite with the key of server 2.4.4 and a random capability, CLI 3.0 (with probe): client ${c2.listening ? 'started' : 'NOT started'} (${c2.ms} ms); ${c2.listening ? describe(Buffer.from('x'.repeat(100)), await roundtrip(c2.port, Buffer.from('x'.repeat(100)), 6000)) : errLine(c2.cli.text)}`)
      c2.cli.kill()
      const port = await free()
      const c3 = new Proc('B3-client3-forged-api', ['-r', PRE3, path.join(HERE, 'client3-api.mjs'), forged, String(port)], env)
      const ok = await c3.waitFor(/READY/, 20000)
      await sleep(1000)
      if (ok) { const p = rnd(1000); say(`B3 (b, wire level) same key, API 3.0 without probe (only the 33 B tunnel header + payload): ${describe(p, await roundtrip(port, p, 6000))}`) }
      else say(`B3: API client did not start: ${c3.tail(3)}`)
      c3.kill()
    }
    srv.kill(); await sleep(500)
  }
}

process.on('SIGINT', () => { for (const p of procs) p.kill(); process.exit(1) })
try { await main() } catch (e) { say('HARNESS ERROR: ' + (e.stack || e)) } finally {
  for (const p of procs) p.kill()
  await sleep(800)
  for (const p of procs) if (!p.exited) p.child.kill('SIGKILL')
  fs.writeFileSync(path.join(LOGS, 'result.txt'), results.join('\n') + '\n')
  process.exit(0)
}
