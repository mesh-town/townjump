// Version difference, UDX layer, second pass (D7 RTT/TLP, D9 MTU probe and change_remote): a raw peer on 127.0.0.1 against udx-native 1.21.3 (new) and 1.19.2 (old).
// The number, sizes and fields of packets are exact; moments are rounded to 100 ms (tolerance ±50). The shared test rig is udx-rig.mjs.
// JS: udx-native/index.js -> libudx (udx.c: schedule_loss_probe, mtu_probeify_packet, udx_stream_change_remote). Output: testvectors/deltas/udx-probes.json
import { createRequire } from 'node:module'
import dgram from 'node:dgram'
import { writeVectors } from './lib.mjs'
import { VERSIONS, T, round, sleep, hdr, parse, rig, strip } from './udx-rig.mjs'
const require = createRequire(import.meta.url)
const until = async (cond, ms = 3000) => { const t = Date.now(); while (!cond() && Date.now() - t < ms) await sleep(2) }

// D7: the peer acks DATA right in the socket handler (the RTT measurement on localhost is sometimes 0 ms), then goes silent on the last one; when does its retransmission (the TLP probe packet) leave?
// The old libudx on a 0 measurement reads srtt == 0 as "no measurement" and sets TLP to 1000 ms, the new one rounds the measurement up to 1 ms (12e5c39). A 0 measurement does not always happen, hence many attempts and only boolean outcomes.
async function tlpTrial(UDX) {
  const r = await rig(UDX)
  let ackAll = true
  r.raw.on('message', (m) => { const p = parse(m); if (ackAll && p.type & T.DATA) r.send(hdr(0, { ack: p.seq + 1 })) })
  for (let i = 0; i < 3; i++) { r.s.write(Buffer.from('warm' + i)); await sleep(40) }
  ackAll = false; r.got.length = 0
  r.s.write(Buffer.from('last')); await sleep(700)
  const dup = r.got.filter((p) => p.seq === r.got[0].seq)
  const result = dup.length > 1 && dup[1].t - dup[0].t < 20 ? 'fast' : dup.length === 1 ? 'none within 700 ms' : 'other'
  await r.close(); return result
}
async function tlp(UDX) {
  const K = 20, results = []
  for (let i = 0; i < K; i++) results.push(await tlpTrial(UDX))
  const n = (x) => results.filter((v) => v === x).length
  // thresholds with a margin: the new version without retransmission in 700 ms has measurements 0-3 of 20, the old one 6-8 of 20
  return { trials: K, noRetransmitWithin700msInAtLeast20pctOfTrials: n('none within 700 ms') >= K / 5 }
}

// D9: the peer cumulatively acks every DATA; JS writes 400 KB. Which datagrams are longer than the base ones and have data_offset != 0?
async function mtuProbe(UDX) {
  const r = await rig(UDX)
  r.raw.on('message', (m) => { const p = parse(m); if (p.type & T.DATA) r.send(hdr(0, { ack: p.seq + 1 })) })
  r.s.write(Buffer.alloc(3000000, 0x22))
  await sleep(4000)
  const data = r.got.filter((p) => p.type & T.DATA)
  const lens = {}; for (const p of data) lens[p.len] = (lens[p.len] || 0) + 1
  const probes = data.filter((p) => p.off !== 0).map((p) => ({ off: p.off, len: p.len, headerBeforeSeq: p.hex.slice(0, 24), ackField: p.hex.slice(32), paddingAllZero: /^0*$/.test(p.hex.slice(40, 40 + 2 * p.off)), payloadBytes: p.len - 20 - p.off }))
  const out = { manyDataPackets: data.length > 1000,  maxLen: Math.max(...data.map((p) => p.len)), probes }
  await r.close(); return out
}

// D9: the stream is connected to peer 1, after an acked DATA it moves to peer 2 (another port, remote_id 9). What goes where?
async function changeRemote(UDX) {
  const r = await rig(UDX)
  const raw2 = dgram.createSocket('udp4'); const got2 = []; const t0 = Date.now()
  raw2.on('message', (m) => got2.push({ t: Date.now() - t0, ...parse(m) }))
  await new Promise((res) => raw2.bind(0, '127.0.0.1', res))
  r.s.write(Buffer.from('one'))
  await until(() => r.got.some((p) => p.type & T.DATA))
  const first = r.got.find((p) => p.type & T.DATA)
  r.send(hdr(0, { ack: first.seq + 1 })); await sleep(100)
  const at1 = r.got.length
  const sock = r.s.socket ?? null
  let changed
  try { changed = r.s.changeRemote(r.s.socket, 9, raw2.address().port, '127.0.0.1'); if (changed && changed.then) await changed; changed = 'ok' } catch (e) { changed = 'error: ' + e.message }
  r.s.write(Buffer.from('two')); await sleep(400)
  raw2.send(hdr(0, { rid: 5, ack: 2 }), r.s.socket.address().port, '127.0.0.1'); await sleep(100)
  const to1 = r.got.slice(at1).map(strip), to2 = got2.map(strip)
  const out = { changeRemote: changed, apiPresent: typeof r.s.changeRemote === 'function', firstPeerAfterChange: to1, secondPeerAfterChange: to2 }
  await r.close(); raw2.close(); return out
}

// D9: change_remote with an unacked DATA in flight (libudx be51b3a: the seq_of_change switch point, defer_change). The old peer acks after 300 ms.
async function changeRemoteInflight(UDX) {
  const r = await rig(UDX)
  const raw2 = dgram.createSocket('udp4'); const got2 = []
  raw2.on('message', (m) => got2.push({ t: Date.now() - r.t0, ...parse(m) }))
  await new Promise((res) => raw2.bind(0, '127.0.0.1', res))
  r.s.write(Buffer.from('one')); await until(() => r.got.some((p) => p.type & T.DATA))
  r.send(hdr(0, { ack: 1 })); await sleep(80)
  r.got.length = 0
  const t = Date.now() - r.t0
  r.s.write(Buffer.from('two')); await sleep(10)
  r.s.changeRemote(r.s.socket, 9, raw2.address().port, '127.0.0.1')
  r.s.write(Buffer.from('three')); await sleep(300)
  r.send(hdr(0, { ack: 2 })); await sleep(1200)
  const line = (peer) => (p) => ({ peer, approxMs: round(p.t - t), rid: p.rid, type: p.type, seq: p.seq, len: p.len })
  // retransmissions and their time float by 100 ms: the file gets only distinct (peer, seq, rid, len), without time
  const uniq = new Map(); for (const p of [...r.got.map(line('old peer')), ...got2.map(line('new peer'))]) uniq.set(`${p.peer}/${p.seq}`, { peer: p.peer, rid: p.rid, type: p.type, seq: p.seq, len: p.len })
  const timeline = [...uniq.values()].sort((a, b) => a.seq - b.seq || a.peer.localeCompare(b.peer))
  const first = (arr, seq) => arr.filter((p) => p.type & T.DATA && p.seq === seq).length
  const out = { distinctPackets: timeline, dataSeq1ToOldPeer: first(r.got, 1) > 0, dataSeq1ToNewPeer: first(got2, 1) > 0, dataSeq2ToOldPeer: first(r.got, 2) > 0, dataSeq2ToNewPeer: first(got2, 2) > 0 }
  await r.close(); raw2.close(); return out
}

const only = process.argv.slice(2)
const scenarios = { tlp, mtuProbe, changeRemote, changeRemoteInflight }
const out = {}
for (const [ver, dir] of Object.entries(VERSIONS)) {
  const UDX = require(dir); out[ver] = {}
  for (const [n, f] of Object.entries(scenarios)) { if (only.length && !only.includes(n)) continue; try { out[ver][n] = await f(UDX) } catch (e) { out[ver][n] = { error: String(e.message) } } }
}
if (only.length) { console.log(JSON.stringify(out, null, 1)); process.exit(0) }
console.log(writeVectors('deltas', 'udx-probes', { note: 'Raw UDP peer against a JS udx-native stream: new = 1.21.3, old = 1.19.2. approxMs rounded to 100 ms. Stream id 5, remote_id 7.', results: out }, []))
process.exit(0)
