// Layer 3: hyperdht codecs (m.*) and dht-rpc request/response headers. Input — fixed values, output — bytes and parsing.
// JS: hyperdht/lib/messages.js, dht-rpc/lib/io.js:Request/_sendReply/_encodeRequest. Output: testvectors/hyperdht/messages.json, testvectors/dht-rpc/header.json
import { req, writeVectors, hex } from './lib.mjs'
const c = req('compact-encoding')
const m = req('hyperdht/lib/messages.js')
const IO = req('dht-rpc/lib/io.js')

const K = (n) => Buffer.alloc(32, n)
const S = (n) => Buffer.alloc(64, n)
const A = (h, p) => ({ host: h, port: p })
const norm = (v) => v instanceof Uint8Array ? hex(v) : Array.isArray(v) ? v.map(norm) : v && typeof v === 'object' ? Object.fromEntries(Object.entries(v).map(([k, x]) => [k, norm(x)])) : v
const jsonIn = norm
function row(label, codec, input) {
  const r = { label, input: jsonIn(input) }
  try {
    const buf = c.encode(codec, input)
    r.encoded = hex(buf)
    r.decoded = norm(c.decode(codec, buf))
  } catch (e) { r.error = e.message }
  return r
}
function rawRow(label, codec, bufHex) {
  const r = { label, encoded: bufHex }
  try { r.decoded = norm(c.decode(codec, Buffer.from(bufHex, 'hex'))) } catch (e) { r.error = e.message }
  return r
}

const relay = { relayAddress: A('127.0.0.1', 49737), peerAddress: A('10.0.0.2', 4000) }
const hpInfo = { id: 7, relays: [relay] }
const noise = Buffer.from('00010203', 'hex')
const handshake = [
  ['mode 0 bare', { mode: 0, noise, peerAddress: null, relayAddress: null }],
  ['mode 1 peerAddress', { mode: 1, noise, peerAddress: A('192.168.1.5', 5555), relayAddress: null }],
  ['mode 2 relayAddress', { mode: 2, noise, peerAddress: null, relayAddress: A('127.0.0.1', 49737) }],
  ['mode 3 both', { mode: 3, noise, peerAddress: A('192.168.1.5', 5555), relayAddress: A('127.0.0.1', 49737) }],
  ['empty noise', { mode: 0, noise: Buffer.alloc(0), peerAddress: null, relayAddress: null }],
].map(([l, i]) => row(l, m.handshake, i))

const base = { error: 0, firewall: 0 }
const payload = [
  ['bare', base],
  ['error 3 firewall 2', { error: 3, firewall: 2 }],
  ['holepunch', { ...base, holepunch: hpInfo }],
  ['addresses4', { ...base, addresses4: [A('1.2.3.4', 80), A('5.6.7.8', 65535)] }],
  ['addresses6', { ...base, addresses6: [A('::1', 4000), A('2001:db8::7', 1)] }],
  ['udx reusableSocket', { ...base, udx: { reusableSocket: true, id: 5, seq: 0 } }],
  ['udx not reusable, big id', { ...base, udx: { reusableSocket: false, id: 70000, seq: 3 } }],
  ['secretStream', { ...base, secretStream: { version: 1 } }],
  ['relayThrough', { ...base, relayThrough: { version: 1, publicKey: K(0xaa), token: K(0xbb) } }],
  ['relayAddresses', { ...base, relayAddresses: [A('127.0.0.1', 49737)] }],
  ['all flags', { error: 0, firewall: 1, holepunch: hpInfo, addresses4: [A('1.2.3.4', 80)], addresses6: [A('::1', 4000)], udx: { reusableSocket: true, id: 5, seq: 0 }, secretStream: { version: 1 }, relayThrough: { version: 1, publicKey: K(0xaa), token: K(0xbb) }, relayAddresses: [A('127.0.0.1', 49737)] }],
].map(([l, i]) => row('noisePayload ' + l, m.noisePayload, i))
payload.push(rawRow('noisePayload version 2 (returned as-is, rest ignored)', m.noisePayload, '0200ff'))

const holepunch = [
  ['no peerAddress', { mode: 0, id: 1, payload: Buffer.from('aabb', 'hex'), peerAddress: null }],
  ['peerAddress', { mode: 2, id: 300, payload: Buffer.alloc(0), peerAddress: A('8.8.4.4', 9999) }],
].map(([l, i]) => row('holepunch ' + l, m.holepunch, i))

const hpp = { error: 0, firewall: 0, round: 0 }
const holepunchPayload = [
  ['bare', hpp],
  ['connected', { ...hpp, connected: true }],
  ['punching round 2', { ...hpp, punching: true, round: 2, firewall: 3 }],
  ['addresses', { ...hpp, addresses: [A('1.2.3.4', 80), A('5.6.7.8', 81)] }],
  ['remoteAddress', { ...hpp, remoteAddress: A('9.9.9.9', 9) }],
  ['token', { ...hpp, token: K(0x11) }],
  ['remoteToken', { ...hpp, remoteToken: K(0x22) }],
  ['all', { error: 1, firewall: 2, round: 3, connected: true, punching: true, addresses: [A('1.2.3.4', 80)], remoteAddress: A('9.9.9.9', 9), token: K(0x11), remoteToken: K(0x22) }],
].map(([l, i]) => row('holepunchPayload ' + l, m.holepunchPayload, i))

const peerA = { publicKey: K(1), relayAddresses: [A('127.0.0.1', 49737)] }
const peerB = { publicKey: K(2), relayAddresses: [] }
const peers = [
  row('peer with relay', m.peer, peerA),
  row('peer no relays', m.peer, peerB),
  row('peers 2', m.peers, [peerA, peerB]),
  row('peers 0', m.peers, []),
]
const rawReply = (ps, bump) => ({ peers: ps.map((p) => c.encode(m.peer, p)), bump })
const lookup = [
  ['2 peers bump 0', rawReply([peerA, peerB], 0)],
  ['0 peers bump 5', rawReply([], 5)],
].map(([l, i]) => row('lookupRawReply ' + l, m.lookupRawReply, i))
lookup.push(rawRow('lookupRawReply without bump byte (decode gives 0)', m.lookupRawReply, '00'))

const announce = [
  ['empty', {}],
  ['peer', { peer: peerA }],
  ['refresh', { refresh: K(3) }],
  ['signature', { signature: S(4) }],
  ['bump', { bump: 9 }],
  ['peer refresh signature bump', { peer: peerA, refresh: K(3), signature: S(4), bump: 300 }],
].map(([l, i]) => row('announce ' + l, m.announce, i))

writeVectors('hyperdht', 'messages', { note: 'encoded = hyperdht/lib/messages.js on a fixed input (Buffer as hex); decoded = parsing of the same bytes. Mutable signatures and pluginRequest — in holesail/dht-record-signing.json and deltas/codec.json.', handshake, noisePayload: payload, holepunch, holepunchPayload, peers, lookupRawReply: lookup, announce }, ['hyperdht', 'compact-encoding'])

// ---- dht-rpc: request and reply header. Real Request/IO.createRequest on a fake io, the socket collects the bytes ----
const sent = []
const sock = { trySend(buf, port, host, ttl) { sent.push({ hex: hex(buf), port, host, ttl }) } }
const ids = { self: K(0xee), token: K(0xdd) }
const nodes = [A('127.0.0.1', 49737), A('10.1.2.3', 6000)]
function fakeIo(ephemeral) {
  return { _destroying: null, _tid: 0, firewalled: false, serverSocket: sock, clientSocket: sock, inflight: [], ephemeral, stats: { commands: new Array(16).fill(0).map(() => ({ tx: 0 })), requests: { active: 0, total: 0 } }, table: { id: ids.self, closest: () => nodes }, token: () => ids.token }
}
const reqRows = []
const to = A('127.0.0.1', 49001)
const cases = [
  ['ping: internal, no target, no value, no token, ephemeral', { command: 0, internal: true, target: null, value: null, token: null, ephemeral: true, tid: 0 }],
  ['non-ephemeral adds id (flag 1)', { command: 0, internal: true, target: null, value: null, token: null, ephemeral: false, tid: 1 }],
  ['token (flag 2)', { command: 1, internal: true, target: K(9), value: null, token: ids.token, ephemeral: true, tid: 255 }],
  ['external command, value', { command: 10, internal: false, target: null, value: Buffer.from('aabbcc', 'hex'), token: null, ephemeral: true, tid: 256 }],
  ['all flags', { command: 4, internal: true, target: K(9), value: Buffer.from('01', 'hex'), token: ids.token, ephemeral: false, tid: 65535 }],
]
for (const [label, o] of cases) {
  const io = fakeIo(o.ephemeral)
  io._tid = o.tid
  const r = IO.prototype.createRequest.call(io, to, o.token, o.internal, o.command, o.target, o.value, null, 0)
  const bytes = r._encodeRequest(o.token, o.value, to, sock)
  const state = { start: 1, end: bytes.byteLength, buffer: bytes } // byte 0 (type and version) is already taken off by io parsing
  const from = { id: null, host: '127.0.0.1', port: 49002 }
  const d = r.constructor.decode(io, sock, from, state)
  reqRows.push({ label, input: norm({ ...o, to }), encoded: hex(bytes), decoded: d && norm({ tid: d.tid, to: d.to, token: d.token, internal: d.internal, command: d.command, target: d.target, value: d.value }) })
}
const replyRows = []
const replyCases = [
  ['ok, no value, token, no closer nodes (target null)', { target: null, error: 0, value: null, token: true, closer: true, ephemeral: true }],
  ['ok, value, no token, closer nodes (target set)', { target: K(9), error: 0, value: Buffer.from('aa', 'hex'), token: false, closer: true, ephemeral: true }],
  ['error 1 UNKNOWN_COMMAND, no token', { target: null, error: 1, value: null, token: false, closer: false, ephemeral: true }],
  ['error 2, id (non-ephemeral), closer nodes', { target: K(9), error: 2, value: null, token: false, closer: true, ephemeral: false }],
  ['everything', { target: K(9), error: 3, value: Buffer.from('bbcc', 'hex'), token: true, closer: true, ephemeral: false }],
]
for (const [label, o] of replyCases) {
  const io = fakeIo(o.ephemeral)
  io._tid = 258
  const r = IO.prototype.createRequest.call(io, to, null, true, 4, o.target, null, null, 0)
  sent.length = 0
  r._sendReply(o.error, o.value, o.token, o.closer, { host: '127.0.0.1', port: 49002 }, sock)
  replyRows.push({ label, tid: 258, from: { host: '127.0.0.1', port: 49002 }, input: norm(o), closerNodes: norm(nodes), encoded: sent[0]?.hex ?? null })
}
writeVectors('dht-rpc', 'header', { note: 'Request: Request._encodeRequest, reply: Request._sendReply on a fake io (node id 0xee*32, token 0xdd*32, closest nodes fixed); decoded — Request.decode of the same bytes. Byte 0: 0x03 request, 0x13 reply; byte 1: flags.', requests: reqRows, replies: replyRows }, ['dht-rpc'])
process.exit(0)
