// Measurement of the steady-state holesail tunnel speed on a large volume: client→server pairs {js,rust}², directions up/down.
// Run: node interop/bench-big.mjs --size 256 --runs 3 --rust-bin <binary> [--pairs js-js,js-rust,rust-js,rust-rust] [--dir up|down|both]
// Each transfer gets its own testnet and its own tunnel (private mode). First 1 MiB of warm-up (DHT connection, handshake),
// the stopwatch starts after the warm-up is received and stops on the last byte at the receiver; sha256 of the whole stream is checked afterwards.
import crypto from 'node:crypto'
import fs from 'node:fs'
import net from 'node:net'
import os from 'node:os'
import path from 'node:path'
import { parseBannerUrl, BANNER_WAIT } from './lib/banner.mjs'
import { childEnv, commandFor, JS_NODE_MODULES, resolveRustBin, ROOT_DIR } from './lib/impl.mjs'
import { freeTcpPort } from './lib/ports.mjs'
import { Proc, sleep } from './lib/proc.mjs'
import { startTestnet } from './lib/testnet.mjs'

const MIB = 1 << 20
const CHUNK = 256 * 1024
const READY_MS = 30_000
const base = crypto.randomBytes(16 * MIB) // the stream is this buffer in a loop; the content is random, checked by the full sha256

function parse(argv) {
  const o = { size: 256, runs: 3, rustBin: process.env.TOWNJUMP_BIN || null, pairs: ['js-js', 'js-rust', 'rust-js', 'rust-rust'], dir: 'both', timeout: 180, settle: 1500 }
  const keys = { '--size': (v) => (o.size = Number(v)), '--runs': (v) => (o.runs = Number(v)), '--rust-bin': (v) => (o.rustBin = v), '--pairs': (v) => (o.pairs = v.split(',')), '--dir': (v) => (o.dir = v), '--timeout': (v) => (o.timeout = Number(v)) }
  for (let i = 0; i < argv.length; i++) (keys[argv[i]] ?? (() => { throw new Error(`unknown flag ${argv[i]}`) }))(argv[++i])
  return o
}

const chunkAt = (off, len) => { const s = off % base.length; return base.subarray(s, Math.min(s + len, base.length)) }
const expectedHash = (total) => { const h = crypto.createHash('sha256'); for (let o = 0; o < total;) { const c = chunkAt(o, Math.min(CHUNK, total - o)); h.update(c); o += c.length } return h.digest('hex') }

async function pump(sock, from, to) {
  for (let off = from; off < to;) {
    const c = chunkAt(off, Math.min(CHUNK, to - off))
    off += c.length
    if (!sock.write(c)) await new Promise((r) => sock.once('drain', r))
  }
}

// The receiver collects chunks without processing (the hash is computed after the stopwatch stops); onGot(n) is called on every chunk.
function receiver(sock, total) {
  const st = { got: 0, chunks: [], warmAt: null, doneAt: null }
  st.warm = new Promise((res) => { st.warmRes = res })
  st.done = new Promise((res) => { st.doneRes = res })
  sock.on('data', (d) => {
    st.chunks.push(d)
    st.got += d.length
    if (st.warmAt === null && st.got >= MIB) { st.warmAt = process.hrtime.bigint(); st.warmRes() }
    if (st.doneAt === null && st.got >= total) { st.doneAt = process.hrtime.bigint(); st.doneRes() }
  })
  return st
}

const within = (p, ms, what) => new Promise((resolve, reject) => { const t = setTimeout(() => reject(new Error(`${what}: timed out after ${ms / 1000} s`)), ms); p.then(resolve, reject).finally(() => clearTimeout(t)) })

async function transfer(ctx, client, server, dir, sizeMib) {
  const procs = []; const socks = []
  const spawn = (impl, role, argv) => { const { cmd, args } = commandFor(impl, argv, ctx.rustBin); const p = new Proc(`${impl} ${role}`, cmd, args, { env: childEnv(testnet.bootstrap) }); procs.push(p); return p }
  const total = MIB + sizeMib * MIB
  const testnet = await startTestnet(4, path.join(ctx.outDir, 'testnet.log'))
  procs.push(testnet.proc)
  let rx
  const t0 = process.hrtime.bigint()
  try {
    const sinkPort = await freeTcpPort()
    let srvRes; const srvSock = new Promise((r) => { srvRes = r })
    const sink = net.createServer((s) => { s.on('error', () => {}); socks.push(s); srvRes(s) }).listen(sinkPort, '127.0.0.1')
    procs.push({ stop: () => new Promise((r) => { sink.close(() => r()); sink.closeAllConnections?.() }) })
    const phrase = `bench-${crypto.randomBytes(16).toString('hex')}`
    const srv = spawn(server, 'server', ['--live', String(sinkPort), '--log', '1', '--key', phrase])
    await srv.waitFor(BANNER_WAIT, READY_MS)
    await srv.waitFor(/Server listening on key/, READY_MS)
    const url = parseBannerUrl(srv.text).url
    const port = await freeTcpPort()
    const cli = spawn(client, 'client', [url, '--port', String(port), '--log', '1'])
    await cli.waitFor(new RegExp(`Proxy listening on 127\\.0\\.0\\.1:${port}`), READY_MS)
    await sleep(ctx.settle)
    const cliSock = net.connect(port, '127.0.0.1'); cliSock.on('error', () => {}); socks.push(cliSock)
    const srvS = await within(srvSock, READY_MS, 'server got no connection from the tunnel')
    const [tx, rcv] = dir === 'up' ? [cliSock, srvS] : [srvS, cliSock]
    rx = receiver(rcv, total)
    const setupMs = Number(process.hrtime.bigint() - t0) / 1e6
    const dead = new Promise((_, rej) => { for (const s of [tx, rcv]) s.on('close', () => rx.doneAt === null && rej(new Error('socket closed before the transfer ended'))) })
    dead.catch(() => {})
    await within(Promise.race([dead, (async () => { await pump(tx, 0, MIB); await rx.warm })()]), ctx.timeout * 1000, 'warm-up') // 1 MiB, wait for it to be received
    const start = process.hrtime.bigint()
    await within(Promise.race([dead, (async () => { await pump(tx, MIB, total); await rx.done })()]), ctx.timeout * 1000, 'transfer')
    const secs = Number(rx.doneAt - start) / 1e9
    const actual = crypto.createHash('sha256'); for (const c of rx.chunks) actual.update(c)
    if (rx.got !== total) throw new Error(`received ${rx.got} bytes, expected ${total}`)
    if (actual.digest('hex') !== expectedHash(total)) throw new Error('stream sha256 mismatch')
    return { mibs: sizeMib / secs, secs, setupMs }
  } catch (e) {
    const got = rx ? `; at the receiver ${(Math.max(rx.got - MIB, 0) / MIB).toFixed(1)} of ${sizeMib} MiB` : ''
    throw new Error(`${e.message}${got}\n${procs.filter((p) => p.tail).map((p) => p.tail(4)).join('\n')}`)
  } finally {
    socks.forEach((s) => s.destroy())
    await Promise.all(procs.map((p) => p.stop()))
    const alive = procs.filter((p) => p.child && p.child.exitCode === null && p.child.signalCode === null && isAlive(p.child.pid))
    if (alive.length) console.log(`ORPHANS: ${alive.map((p) => `${p.name}:${p.child.pid}`).join(' ')}`)
  }
}

const isAlive = (pid) => { try { process.kill(pid, 0); return true } catch { return false } }
const median = (a) => { const s = [...a].sort((x, y) => x - y); const m = s.length >> 1; return s.length % 2 ? s[m] : (s[m - 1] + s[m]) / 2 }
const fmt = (x) => x.toFixed(2)

async function main() {
  const o = parse(process.argv.slice(2))
  const rustBin = resolveRustBin(o.rustBin)
  if (o.pairs.some((p) => p.includes('rust')) && !rustBin) throw new Error('Rust binary not found: --rust-bin')
  const ctx = { rustBin, timeout: o.timeout, settle: o.settle, outDir: path.join(ROOT_DIR, '.tmp/bench-out') }
  fs.mkdirSync(ctx.outDir, { recursive: true })
  const dirs = o.dir === 'both' ? ['up', 'down'] : [o.dir]
  console.log(`bench: size ${o.size} MiB, runs ${o.runs}, pairs ${o.pairs}, directions ${dirs}; JS ${JS_NODE_MODULES}; Rust ${rustBin ?? 'none'}; load ${os.loadavg().map(fmt)}`)
  const res = new Map()
  for (let r = 0; r < o.runs; r++) {
    const order = o.pairs.map((_, i) => o.pairs[(i + r) % o.pairs.length]) // the pair order shifts on every run
    for (const pair of order) for (const dir of (r % 2 ? [...dirs].reverse() : dirs)) {
      const [client, server] = pair.split('-'); const key = `${pair} ${dir}`
      if (!res.has(key)) res.set(key, { ok: [], fail: [] })
      try {
        const t = await transfer(ctx, client, server, dir, o.size)
        res.get(key).ok.push(t.mibs)
        console.log(`run ${r + 1} ${key.padEnd(14)} ok   ${fmt(t.mibs).padStart(7)} MiB/s  (${t.secs.toFixed(1)} s, setup ${(t.setupMs / 1000).toFixed(1)} s) load ${os.loadavg()[0].toFixed(1)}`)
      } catch (e) {
        res.get(key).fail.push(e.message)
        console.log(`run ${r + 1} ${key.padEnd(14)} FAIL ${e.message.split('\n')[0]}\n${e.message.split('\n').slice(1).join('\n')}`)
      }
    }
  }
  console.log(`\n| pair | dir. | median MiB/s | min–max | volume | ok/FAIL |\n|---|---|---|---|---|---|`)
  for (const [key, v] of res) {
    const [pair, dir] = key.split(' ')
    const stat = v.ok.length ? `${fmt(median(v.ok))} | ${fmt(Math.min(...v.ok))}–${fmt(Math.max(...v.ok))}` : '— | —'
    console.log(`| ${pair} | ${dir} | ${stat} | ${o.size} MiB | ${v.ok.length}/${v.fail.length} |`)
  }
  console.log(`load after: ${os.loadavg().map(fmt)}`)
  process.exitCode = [...res.values()].some((v) => v.fail.length) ? 1 : 0
}

main().catch((e) => { console.error(`startup error: ${e.message}`); process.exit(2) })
