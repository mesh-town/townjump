// secret-stream layer: frames of @hyperswarm/secret-stream 6.9.2 with fixed keys, without a network (two in-memory streams).
// JS: @hyperswarm/secret-stream/index.js, lib/handshake.js, noise-handshake/noise.js, hyperdht/lib/noise-wrap.js, sodium-secretstream/index.js.
// Output: testvectors/secret-stream/{xx-inband,ik-hyperdht,negative}.json
// Substitutions (foreign code is not edited, only the sources of randomness are replaced; the keys and handshake bytes stay real):
//  1. Noise ephemeral key: noise.e is set before the first send (otherwise crypto_sign_keypair);
//  2. secretstream header (24 bytes): crypto_secretstream_..._init_push of sodium-secretstream is replaced by "header = BLAKE2b-24(key), state through init_pull" — in libsodium the push and pull state is the same by header;
//  3. initial counter of unordered messages: randombytes_buf of sodium secret-stream gives deterministic bytes.
import { createRequire } from 'node:module'
import { createHash } from 'node:crypto'
import { req, NM, writeVectors, hex } from './lib.mjs'

const SS = req('@hyperswarm/secret-stream')
const HS = req('@hyperswarm/secret-stream/lib/handshake.js')
const { Duplex } = req('streamx')
const hcrypto = req('hypercore-crypto')
const NoiseWrap = req('hyperdht/lib/noise-wrap.js')
const c = req('compact-encoding')
const m = req('hyperdht/lib/messages.js')

const ssSodium = createRequire(NM + '/sodium-secretstream/')('sodium-universal')
const realInitPull = ssSodium.crypto_secretstream_xchacha20poly1305_init_pull
ssSodium.crypto_secretstream_xchacha20poly1305_init_push = (state, header, key) => { ssSodium.crypto_generichash(header, key); realInitPull(state, header, key) }
const streamSodium = createRequire(NM + '/@hyperswarm/secret-stream/')('sodium-universal')
let randCall = 0
streamSodium.randombytes_buf = (buf) => { randCall++; buf.fill(0xa0 + randCall) } // 1st call 0xa1.., 2nd 0xa2..; which stream got which — in the JSON ("sendState")

const seed = (n) => Buffer.alloc(32, n)
const sha = (b) => createHash('sha256').update(b).digest('hex')
const plainOf = (len, salt) => Buffer.from(Array.from({ length: len }, (_, i) => (i * 31 + salt) & 255))
const until = async (cond, what) => { for (let i = 0; i < 5000; i++) { if (cond()) return; await new Promise((r) => setImmediate(r)) } throw new Error('timeout: ' + what) }
const kpJson = (kp) => ({ publicKey: hex(kp.publicKey), secretKey: hex(kp.secretKey) })

// a pair of raw streams: everything written by one side arrives at the other in the same chunk; log keeps the chunks in order
function rawPair() {
  const log = []
  let a, b
  const mk = (from, peer) => new Duplex({ write(d, cb) { log.push({ from, hex: hex(d) }); peer().push(Buffer.from(d)); cb(null) }, final(cb) { peer().push(null); cb(null) } })
  a = mk('initiator', () => b); b = mk('responder', () => a)
  const unordered = (from, peer) => (msg) => { log.push({ from, unordered: hex(msg) }); peer().emit('message', Buffer.from(msg)); return true }
  a.send = a.trySend = unordered('initiator', () => b); b.send = b.trySend = unordered('responder', () => a)
  return { a, b, log }
}
const withEphemeral = (s, ephSeed) => { s._handshake.noise.e = SS.keyPair(ephSeed); return hex(s._handshake.noise.e.publicKey) }
const recorder = (s) => { const r = { plain: [], errors: [], messages: [] }; s.on('data', (d) => r.plain.push(hex(d))); s.on('error', (e) => r.errors.push(e.message)); s.on('message', (d) => r.messages.push(hex(d))); return r }
const sendState = (s) => ({ sendSecret: hex(s._sendState.subarray(0, 32)), recvSecret: hex(s._sendState.subarray(32, 64)), counter: hex(s._sendState.subarray(64, 72)), initialCounter: hex(s._sendState.subarray(72)) })

const LENS_I = [1, 2, 15, 16, 17, 255, 256, 1000]
const LENS_R = [1, 100]
const BIG = [65536, 70000] // hash instead of bytes: the third byte of the frame length

// Parsing of the recorded exchange: the chunks of each direction go in order — handshake messages (nHs), the header, the frames.
function classify(log, nHs) {
  const idx = { initiator: 0, responder: 0 }
  return log.filter((e) => !e.unordered).map((e, seq) => {
    const k = idx[e.from]++, need = nHs[e.from]
    const kind = k < need ? 'handshake' : k === need ? 'header' : 'frame'
    const len = parseInt(e.hex.slice(0, 6).match(/../g).reverse().join(''), 16)
    if (len !== e.hex.length / 2 - 3) throw new Error('length prefix mismatch')
    const body = e.hex.slice(6), small = body.length <= 2200 // large bodies — only length and hash
    return { seq, from: e.from, kind, lengthPrefix: e.hex.slice(0, 6), ...(small ? { body } : { bodyBytes: body.length / 2, bodySha256: sha(Buffer.from(body, 'hex')) }), ...(kind === 'header' ? { id: e.hex.slice(6, 70), secretstreamHeader: e.hex.slice(70) } : {}) }
  })
}

async function talk({ sI, sR, rI, rR, log, nHs, filterR }) {
  await Promise.all([sI.opened, sR.opened])
  const frames = { initiator: [], responder: [] }
  for (const n of LENS_I) { const p = plainOf(n, n); sI.write(p); frames.initiator.push({ plainLen: n, plainSalt: n, plain: hex(p) }) }
  sI.sendKeepAlive(); frames.initiator.push({ plainLen: 0, keepAlive: true, plain: '' })
  for (const n of LENS_R) { const p = plainOf(n, 100 + n); sR.write(p); frames.responder.push({ plainLen: n, plainSalt: 100 + n, plain: hex(p) }) }
  for (const n of BIG) sI.write(plainOf(n, 9))
  const big = BIG.map((n) => ({ plainLen: n, plainSalt: 9, plainSha256: sha(plainOf(n, 9)) }))
  const um = [Buffer.from('unordered-one'), Buffer.alloc(0), Buffer.from('third')]
  sI.send(um[0]); sI.send(um[1]); sI.trySend(um[2]); sR.send(Buffer.from('reply'))
  const wantR = LENS_I.length + BIG.length + (filterR ? 0 : 1) // keepalive reaches the read only without a filter
  await until(() => rR.plain.length >= wantR && rI.plain.length >= LENS_R.length && rR.messages.length >= 3 && rI.messages.length >= 1, 'frames')
  const wire = classify(log, nHs)
  const perDir = { initiator: wire.filter((w) => w.from === 'initiator' && w.kind === 'frame'), responder: wire.filter((w) => w.from === 'responder' && w.kind === 'frame') }
  const bigWire = perDir.initiator.splice(LENS_I.length + 1) // keepalive (1) + LENS_I go before BIG
  frames.initiator.forEach((f, i) => Object.assign(f, { lengthPrefix: perDir.initiator[i].lengthPrefix, ciphertext: perDir.initiator[i].body }))
  frames.responder.forEach((f, i) => Object.assign(f, { lengthPrefix: perDir.responder[i].lengthPrefix, ciphertext: perDir.responder[i].body }))
  big.forEach((f, i) => Object.assign(f, { lengthPrefix: bigWire[i].lengthPrefix, ciphertextBytes: bigWire[i].bodyBytes, ciphertextSha256: bigWire[i].bodySha256 }))
  const unordered = log.filter((e) => e.unordered).map((e) => ({ from: e.from, envelope: e.unordered, counter8: e.unordered.slice(0, 16), ciphertextWithMac: e.unordered.slice(16) }))
  return { wire, frames, bigFrames: big, unorderedSent: { plains: [...um.map(hex), hex(Buffer.from('reply'))], envelopes: unordered }, received: { responderPlain: rR.plain.map((p) => p.length / 2), responderPlainMatch: LENS_I.every((n, i) => rR.plain[i] === hex(plainOf(n, n))), initiatorPlainMatch: LENS_R.every((n, i) => rI.plain[i] === hex(plainOf(n, 100 + n))), responderUnordered: rR.messages, initiatorUnordered: rI.messages, responderGotEmptyFrame: filterR ? false : rR.plain[LENS_I.length] === '' } }
}

const names = () => { const [i, r, s] = hcrypto.namespace('hyperswarm/secret-stream', 3); return { NS_INITIATOR: hex(i), NS_RESPONDER: hex(r), NS_SEND: hex(s) } }
const idOf = (hash, init) => hex(SS.id(Buffer.from(hash, 'hex'), init))

// 1. Handshake inside the stream: Noise XX (the default pattern), then the header and frames
async function xxInband() {
  const kpI = SS.keyPair(seed(1)), kpR = SS.keyPair(seed(2)), ephI = seed(3), ephR = seed(4)
  const spy = [], orig = HS.prototype._return
  HS.prototype._return = function (d) { const r = orig.call(this, d); if (r.tx) spy.push({ isInitiator: this.isInitiator, tx: hex(r.tx), rx: hex(r.rx), hash: hex(r.hash), remotePublicKey: hex(r.remotePublicKey) }); return r }
  randCall = 0
  const { a, b, log } = rawPair()
  const sI = new SS(true, a, { keyPair: kpI }), sR = new SS(false, b, { keyPair: kpR })
  const ephPub = { initiator: withEphemeral(sI, ephI), responder: withEphemeral(sR, ephR) }
  const rI = recorder(sI), rR = recorder(sR)
  await until(() => spy.length === 2, 'handshake')
  HS.prototype._return = orig
  const t = await talk({ sI, sR, rI, rR, log, nHs: { initiator: 2, responder: 1 }, filterR: false })
  const ss = { initiator: sendState(sI), responder: sendState(sR) }
  const hash = spy[0].hash
  const chunks = log.filter((e) => e.from === 'initiator' && !e.unordered).map((e) => e.hex)
  const replay = await replayResponder(kpR, ephR, chunks, t.frames.initiator, 'XX')
  return { note: 'Two real NoiseSecretStream instances with the default XX pattern, connected by raw in-memory streams. Keys: seed 1 (initiator), 2 (responder) in crypto_sign_seed_keypair; ephemeral seeds 3 and 4. The "wire" chunks go in write order, each with 3 length bytes (uint24 little-endian) + body. handshake: Noise messages (msg1 of the initiator, msg2 of the responder, msg3 of the initiator); header: stream id (32 bytes, of the sending side) + secretstream header (24 bytes); frame: secretstream ciphertext (plain + 17 bytes). The secretstream header is random in real operation, here it is BLAKE2b-24 of the direction key (substitution 2 in the script header).', patterns: { name: 'XX', prologue: '' }, keys: { initiator: { seed: hex(seed(1)), ...kpJson(kpI) }, responder: { seed: hex(seed(2)), ...kpJson(kpR) } }, ephemeral: { initiatorSeed: hex(ephI), responderSeed: hex(ephR), publicKeys: ephPub }, noiseOutputs: spy, handshakeHash: hash, streamIds: { initiator: idOf(hash, true), responder: idOf(hash, false) }, namespaces: names(), sendState: ss, ...t, chunkingReplay: replay }
}

// The same responder gets the recorded initiator chunks, cut differently: the parser must not depend on packet boundaries
async function replayResponder(kpR, ephR, chunks, expectFrames, pattern, hsOpt) {
  const out = []
  for (const mode of ['whole', 'bytewise', 'halves']) {
    const sink = new Duplex({ write(d, cb) { cb(null) } })
    const sR = hsOpt ? new SS(false, sink, { handshake: hsOpt }) : new SS(false, sink, { keyPair: kpR, pattern })
    if (!hsOpt) withEphemeral(sR, ephR)
    const r = recorder(sR)
    const all = Buffer.concat(chunks.map((h) => Buffer.from(h, 'hex')))
    const pieces = mode === 'whole' ? [all] : mode === 'bytewise' ? Array.from(all, (x) => Buffer.from([x])) : chunks.flatMap((h) => { const b = Buffer.from(h, 'hex'), k = b.length >> 1; return [b.subarray(0, k), b.subarray(k)] })
    for (const p of pieces) { sink.push(p); await new Promise((res) => setImmediate(res)) }
    const want = expectFrames.filter((f) => f.plainLen > 0).length
    await until(() => r.plain.filter((x) => x !== '').length >= want, 'replay ' + mode)
    out.push({ mode, chunkSizes: mode === 'whole' ? [all.length] : mode === 'bytewise' ? 'all 1' : 'each recorded chunk split in two halves', plainMatches: expectFrames.filter((f) => f.plainLen > 0 && !f.keepAlive).every((f, i) => r.plain[i] === f.plain) })
    sR.destroy()
  }
  return out
}

// 2. As in hyperdht: the handshake outside (NoiseWrap, Noise IK, prologue NS_PEER_HANDSHAKE), the stream gets { tx, rx, hash, publicKey, remotePublicKey }
async function ikHyperdht() {
  const kpI = SS.keyPair(seed(5)), kpR = SS.keyPair(seed(6)), ephI = seed(7), ephR = seed(8)
  const wI = new NoiseWrap(kpI, kpR.publicKey), wR = new NoiseWrap(kpR)
  wI.handshake.e = SS.keyPair(ephI)
  const pI = { error: 0, firewall: 1, holepunch: null, addresses4: [], addresses6: [], udx: { reusableSocket: false, id: 11, seq: 0 }, secretStream: { version: 1 }, relayThrough: null, relayAddresses: null }
  const pR = { ...pI, firewall: 2, udx: { reusableSocket: true, id: 22, seq: 0 } }
  const msg1 = wI.send(pI); const decodedI = wR.recv(msg1)
  wR.handshake.e = SS.keyPair(ephR)
  const msg2 = wR.send(pR); const decodedR = wI.recv(msg2)
  const hI = wI.final(), hR = wR.final()
  const j = (h) => ({ isInitiator: h.isInitiator, publicKey: hex(h.publicKey), remotePublicKey: hex(h.remotePublicKey), remoteId: hex(h.remoteId), holepunchSecret: hex(h.holepunchSecret), hash: hex(h.hash), rx: hex(h.rx), tx: hex(h.tx) })
  randCall = 0
  const { a, b, log } = rawPair()
  const sI = new SS(true, a, { handshake: hI }), sR = new SS(false, b, { handshake: hR, filterZeroByteMessages: true })
  const rI = recorder(sI), rR = recorder(sR)
  const t = await talk({ sI, sR, rI, rR, log, nHs: { initiator: 0, responder: 0 }, filterR: true })
  const chunks = log.filter((e) => e.from === 'initiator' && !e.unordered).map((e) => e.hex)
  const replay = await replayResponder(kpR, ephR, chunks, t.frames.initiator, 'IK', hR)
  const ctx = { kpI, kpR, hI, hR, log }
  return { ctx, doc: { note: 'Handshake as in hyperdht/lib/noise-wrap.js (Noise IK, prologue NS.PEER_HANDSHAKE, payload — messages.noisePayload), then the streams are created as in hyperdht/lib/connect.js: start(raw, { handshake: final() }). The Noise messages (msg1, msg2) go not over the raw stream but inside dht-rpc; the raw stream carries only the header and frames. The responder has filterZeroByteMessages: true (as with keepAlive > 0): an empty keepalive frame is decrypted but does not reach the read.', keys: { initiator: { seed: hex(seed(5)), ...kpJson(kpI) }, responder: { seed: hex(seed(6)), ...kpJson(kpR) } }, ephemeral: { initiatorSeed: hex(ephI), responderSeed: hex(ephR) }, noise: { pattern: 'IK', prologueNamespace: 'NS.PEER_HANDSHAKE (hyperdht/lib/constants.js)', payloadInitiator: { input: JSON.parse(JSON.stringify(pI)), encoded: hex(c.encode(m.noisePayload, pI)) }, payloadResponder: { input: JSON.parse(JSON.stringify(pR)), encoded: hex(c.encode(m.noisePayload, pR)) }, msg1: hex(msg1), msg2: hex(msg2), msg1DecodedOk: decodedI.udx.id === 11, msg2DecodedOk: decodedR.udx.id === 22 }, final: { initiator: j(hI), responder: j(hR) }, streamIds: { initiator: idOf(hex(hI.hash), true), responder: idOf(hex(hI.hash), false) }, namespaces: names(), sendState: { initiator: sendState(sI), responder: sendState(sR) }, ...t, chunkingReplay: replay } }
}

// 3. Negative cases
async function negative(ctx) {
  const { hR, hI, log } = ctx
  const hdrI = log.find((e) => e.from === 'initiator' && !e.unordered).hex // initiator header: 3 + 56 bytes
  const frame0 = log.filter((e) => e.from === 'initiator' && !e.unordered)[1].hex
  const flip = (h, i) => { const b = Buffer.from(h, 'hex'); b[i] ^= 1; return hex(b) }
  const feed = async (label, input, description) => {
    const sink = new Duplex({ write(d, cb) { cb(null) } })
    const s = new SS(false, sink, { handshake: hR }), r = recorder(s)
    await s.opened
    for (const h of input) { sink.push(Buffer.from(h, 'hex')); await new Promise((res) => setImmediate(res)) }
    await new Promise((res) => setTimeout(res, 30))
    return { label, description, inputChunks: input, plainReceived: r.plain, errors: r.errors, destroyed: s.destroyed || s.destroying }
  }
  const rows = []
  rows.push(await feed('header id wrong', [flip(hdrI, 3)], 'first byte of id changed'))
  rows.push(await feed('header wrong length', ['370000' + hdrI.slice(6, 6 + 110)], 'header length 55 bytes instead of 56 (prefix 0x37)'))
  rows.push(await feed('header body short of prefix', ['380000' + hdrI.slice(6, 6 + 110)], 'prefix promises 56 bytes, 55 arrived: the parser waits for the rest, no error'))
  rows.push(await feed('frame tampered', [hdrI, flip(frame0, frame0.length / 2 - 1)], 'last ciphertext byte of the first frame changed'))
  rows.push(await feed('frame shorter than ABYTES', [hdrI, '0a0000' + '00'.repeat(10)], 'a 10-byte frame (less than 17 authentication bytes)'))
  rows.push(await feed('frame before header', [frame0], 'frame instead of header: the length is not 56'))
  const xx = new SS(false, new Duplex({ write(d, cb) { cb(null) } }), { keyPair: SS.keyPair(seed(2)) }); const rx = recorder(xx)
  xx._rawStream.push(Buffer.from('1f0000' + '00'.repeat(31), 'hex'))
  await new Promise((res) => setTimeout(res, 30))
  rows.push({ label: 'XX msg1 31 bytes', description: 'XX responder got a 31-byte msg1 (32 needed)', inputChunks: ['1f0000' + '00'.repeat(31)], errors: rx.errors, destroyed: xx.destroyed || xx.destroying })
  let ctorErr = null
  try { new SS('yes', null) } catch (e) { ctorErr = e.message }
  rows.push({ label: 'isInitiator not boolean', description: 'new NoiseSecretStream("yes", null)', errors: [ctorErr] })
  const big = async (n) => {
    const { a, b } = rawPair()
    const s = new SS(true, a, { handshake: hI }), r = recorder(s); new SS(false, b, { handshake: hR })
    await s.opened
    const p = Buffer.alloc(n, 7)
    s.write(p)
    await new Promise((res) => setTimeout(res, 300))
    return { label: 'write of ' + n + ' bytes', description: 'frame limit: 3 + n + 17 - 3 <= 16777215, that is n <= 16777198', plainLen: n, errors: r.errors, destroyed: s.destroyed || s.destroying }
  }
  rows.push(await big(16777198), await big(16777199))
  return rows
}

const xx = await xxInband()
writeVectors('secret-stream', 'xx-inband', xx, ['@hyperswarm/secret-stream', 'noise-handshake', 'noise-curve-ed', 'sodium-secretstream', 'sodium-native'])
const ik = await ikHyperdht()
writeVectors('secret-stream', 'ik-hyperdht', ik.doc, ['@hyperswarm/secret-stream', 'noise-handshake', 'sodium-secretstream', 'hyperdht'])
writeVectors('secret-stream', 'negative', { note: 'Responder with ready IK keys (see ik-hyperdht.json) gets corrupted chunks; errors — Error messages from destroy. The write limit is checked with an initiator-responder pair.', cases: await negative(ik.ctx) }, ['@hyperswarm/secret-stream'])
process.exit(0)
