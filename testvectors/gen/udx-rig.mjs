// Shared UDX test rig for the deltas-udx*.mjs generators: a raw UDP peer (Node socket on 127.0.0.1) against a JS udx-native stream 1.21.3 (new) and 1.19.2 (old).
// Packet header 20 bytes LE: [0]=ff [1]=01 [2]=type [3]=data_offset [4..8)=remote_id [8..12)=window [12..16)=seq [16..20)=ack. Neighbor: deltas-udx.mjs, deltas-udx-probes.mjs
import dgram from 'node:dgram'
import path from 'node:path'
import { ROOT, NM } from './lib.mjs'

export const VERSIONS = { new: path.join(NM, 'udx-native'), old: path.join(ROOT, '.tmp/peeroxide-node-theirs/node_modules/udx-native') }
export const T = { DATA: 1, END: 2, SACK: 4, MESSAGE: 8, DESTROY: 16, HEARTBEAT: 32 }
export const round = (ms) => Math.round(ms / 100) * 100
export const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
export const hdr = (type, { rid = 5, win = 0x400000, seq = 0, ack = 0, off = 0 } = {}, body = Buffer.alloc(0)) => {
  const b = Buffer.alloc(20); b[0] = 0xff; b[1] = 1; b[2] = type; b[3] = off
  b.writeUInt32LE(rid, 4); b.writeUInt32LE(win, 8); b.writeUInt32LE(seq, 12); b.writeUInt32LE(ack, 16)
  return Buffer.concat([b, body])
}
export const sack = (blocks) => { const b = Buffer.alloc(8 * blocks.length); blocks.forEach(([s, e], i) => { b.writeUInt32LE(s, 8 * i); b.writeUInt32LE(e, 8 * i + 4) }); return b }
export const parse = (b) => ({ type: b[2], off: b[3], rid: b.readUInt32LE(4), win: b.readUInt32LE(8), seq: b.readUInt32LE(12), ack: b.readUInt32LE(16), len: b.length, hex: b.toString('hex') })

// test rig: a raw peer + a JS stream id 5 connected to remote_id 7 on the peer port
export async function rig(UDX, { keep = 0 } = {}) {
  const raw = dgram.createSocket('udp4')
  const got = []
  const t0 = Date.now()
  raw.on('message', (m) => got.push({ t: Date.now() - t0, ...parse(m) }))
  await new Promise((r) => raw.bind(0, '127.0.0.1', r))
  const u = new UDX(), sock = u.createSocket(); sock.bind(0, '127.0.0.1')
  const s = u.createStream(5)
  const events = []
  s.on('error', (e) => events.push({ t: Date.now() - t0, error: e.code || e.message }))
  s.on('close', () => events.push({ t: Date.now() - t0, close: true }))
  s.on('data', (d) => events.push({ data: d.toString('hex') }))
  s.connect(sock, 7, raw.address().port, '127.0.0.1')
  const send = (pkt) => raw.send(pkt, sock.address().port, '127.0.0.1')
  const close = async () => { try { s.destroy() } catch {} ; raw.close(); sock.close?.() }
  return { raw, s, got, events, send, close, t0 }
}
export const strip = (p) => ({ type: p.type, seq: p.seq, ack: p.ack, len: p.len, hex: p.hex })
