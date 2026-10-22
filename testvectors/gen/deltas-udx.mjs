// Version difference, UDX layer (D2 D3 D4 D6 D7 D8): a raw UDP peer (Node socket on 127.0.0.1) against a JS udx-native stream 1.21.3 (reference) and 1.19.2 (old).
// Header bytes and the number/composition of packets are exact; moments are rounded to 100 ms (tolerance ±50), otherwise the output will not repeat byte for byte.
// JS: udx-native/index.js -> libudx (udx.c: process_packet, send_probe, udx_sack_is_valid). Samples: testvectors/deltas-js/*.js. Output: testvectors/deltas/udx.json
import { createRequire } from 'node:module'
import { writeVectors } from './lib.mjs'

import { VERSIONS, T, round, sleep, hdr, sack, parse, rig, strip } from './udx-rig.mjs'
const require = createRequire(import.meta.url)

async function packetsOnWire(UDX) { // D2, D3: empty MESSAGE and HEARTBEAT from keepalive
  const a = await rig(UDX); a.s.trySend(Buffer.alloc(0)); await sleep(150)
  const emptyMessage = a.got.map(strip); await a.close()
  const b = await rig(UDX)
  if (typeof b.s.setKeepalive !== 'function') { await b.close(); return { emptyMessage, keepalive300: { error: 'setKeepalive is missing in this version of udx-native' } } }
  b.s.setKeepalive(300); await sleep(1000)
  const heartbeats = b.got.filter((p) => p.type === T.HEARTBEAT); await b.close()
  return { emptyMessage, keepalive300: { firstHeartbeat: heartbeats[0] && strip(heartbeats[0]), countIn1000ms: heartbeats.length, gapsMs: heartbeats.slice(1).map((p, i) => round(p.t - heartbeats[i].t)) } }
}
async function heartbeatReply(UDX) { // D3: the reply to a fed HEARTBEAT (the probe fields are junk, to see that they are ignored)
  const out = []
  for (const [label, opt] of [['plain', {}], ['garbage fields', { win: 0, seq: 999, ack: 12345 }]]) {
    const r = await rig(UDX); r.send(hdr(T.HEARTBEAT, opt)); await sleep(200)
    out.push({ label, sent: hdr(T.HEARTBEAT, opt).toString('hex'), replies: r.got.map(strip) }); await r.close()
  }
  return out
}
async function ackPerData(UDX) { // D8: N DATA -> N ACK; a gap and its close
  const r = await rig(UDX)
  const seqs = [0, 1, 2, 4, 5, 7, 3, 6, 2]
  const steps = []
  for (const seq of seqs) {
    const before = r.got.length
    r.send(hdr(T.DATA, { seq }, Buffer.from([0xa0 + seq]))); await sleep(40)
    steps.push({ sentSeq: seq, sent: hdr(T.DATA, { seq }, Buffer.from([0xa0 + seq])).toString('hex'), replies: r.got.slice(before).map(strip) })
  }
  const delivered = r.events.filter((e) => e.data).map((e) => e.data); await r.close()
  return { steps, deliveredInOrder: delivered }
}
async function strictSack(UDX, label, plan) { // D6: JS sends DATA seq 0..5, the peer sends ACK with blocks; we observe which seq are retransmitted
  const r = await rig(UDX)
  r.s.write(Buffer.alloc(6600, 0x11)) // 6 packets of <=1180 bytes: seq 0..5 (small writes go one at a time: udx-native waits for the write ack)
  await sleep(150)
  const first = r.got.filter((p) => p.type & T.DATA).map((p) => p.seq)
  for (const step of plan) { r.send(hdr(step.blocks.length ? T.SACK : 0, { ack: step.ack ?? 0 }, sack(step.blocks))); await sleep(step.wait ?? 60) }
  r.got.length = 0; await sleep(1500)
  const retransmitted = [...new Set(r.got.filter((p) => p.type & T.DATA).map((p) => p.seq))].sort((a, b) => a - b)
  await r.close()
  return { label, firstDataSeqs: first, plan: plan.map((p) => ({ ack: p.ack ?? 0, blocks: p.blocks })), retransmittedSeqsIn1500ms: retransmitted }
}
async function zwp(UDX) { // D4: the peer announced window 0, JS writes 5 bytes
  const r = await rig(UDX); r.send(hdr(0, { win: 0 })); await sleep(50); r.s.write(Buffer.from('hello'))
  await sleep(7200)
  const pk = r.got.map((p) => ({ approxMs: round(p.t), type: p.type, len: p.len })); await r.close(); return pk
}
async function rto(UDX) { // D7: a silent peer
  const r = await rig(UDX); r.s.write(Buffer.from('hello'))
  await new Promise((res) => { r.s.on('close', res); setTimeout(res, 20000) })
  const out = { packets: r.got.map((p) => ({ approxMs: round(p.t), type: p.type, len: p.len, seq: p.seq })), events: r.events.filter((e) => !e.data).map((e) => ({ approxMs: round(e.t), ...(e.error ? { error: e.error } : { close: true }) })) }
  await r.close(); return out
}
const scenarios = {
  packetsOnWire, heartbeatReply, ackPerData, zwp, rto,
  // blocks start at 0 (or at ack), so that an accepted block moves the queue head: then RTO retransmits not seq 0 but the first unaccepted one
  strictSack_valid: (U) => strictSack(U, 'valid: ack=0, block [0,2)', [{ blocks: [[0, 2]] }]),
  strictSack_endEqSeq: (U) => strictSack(U, 'end == seq: ack=0, block [0,6) (seq=6)', [{ blocks: [[0, 6]] }]),
  strictSack_endAboveSeq: (U) => strictSack(U, 'end > seq: ack=0, block [0,7)', [{ blocks: [[0, 7]] }]),
  strictSack_startBelowAcked: (U) => strictSack(U, 'start < remote_acked: ack=2, block [1,3)', [{ ack: 2, blocks: [[1, 3]] }]),
  strictSack_startAtAcked: (U) => strictSack(U, 'control, start == remote_acked: ack=2, block [2,4)', [{ ack: 2, blocks: [[2, 4]] }]),
  strictSack_startEqEnd: (U) => strictSack(U, 'start == end: ack=0, blocks [0,0) then [0,2)', [{ blocks: [[0, 0], [0, 2]] }]),
  strictSack_endBelowStart: (U) => strictSack(U, 'end < start: ack=0, blocks [5,0) then [0,2)', [{ blocks: [[5, 0], [0, 2]] }]),
  strictSack_mixed: (U) => strictSack(U, 'valid and invalid in one packet: ack=0, blocks [0,2) then [4,9)', [{ blocks: [[0, 2], [4, 9]] }]),
}
const NEW_ONLY = ['strictSack_endBelowStart'] // the old libudx spins a loop of ~2^32 steps on end < start: we do not run it
const only = process.argv.slice(2)
const out = {}
for (const [ver, dir] of Object.entries(VERSIONS)) {
  const UDX = require(dir)
  const names = Object.keys(scenarios).filter((n) => (!only.length || only.includes(n)) && !(ver === 'old' && NEW_ONLY.includes(n)))
  // slow scenarios (rto ~13 s, zwp ~7 s) run in parallel with the fast ones; the fast ones run sequentially
  const slow = names.filter((n) => n === 'rto' || n === 'zwp'), fast = names.filter((n) => !slow.includes(n))
  const res = {}
  await Promise.all([
    (async () => { for (const n of fast) { try { res[n] = await Promise.race([scenarios[n](UDX), sleep(20000).then(() => ({ timeout: true }))]) } catch (e) { res[n] = { error: String(e.message) } } } })(),
    ...slow.map(async (n) => { res[n] = await scenarios[n](UDX) }),
  ])
  out[ver] = Object.fromEntries(names.map((n) => [n, res[n]]))
}
if (only.length) { console.log(JSON.stringify(out, null, 1)); process.exit(0) }
const p = writeVectors('deltas', 'udx', { note: 'Raw UDP peer against a JS udx-native stream: new = 1.21.3 (reference), old = 1.19.2. Packets are whole hex (20-byte LE header: [0]=ff [1]=01 [2]=type [3]=data_offset [4..8)=remote_id [8..12)=window [12..16)=seq [16..20)=ack). approxMs rounded to 100 ms, tolerance ±50. JS stream: id 5, remote_id 7.', versions: { new: '1.21.3', old: '1.19.2' }, results: out }, ['udx-native'])
console.log(p)
process.exit(0)
