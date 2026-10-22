// Holesail layer: the server record in the DHT. (1) pure vectors of the mutable signature and MUTABLE_PUT/MUTABLE_GET encodings;
// (2) a live run of the real HolesailServer.start/put, HolesailClient.get/ping on a local testnet (127.0.0.1) — seq and value step by step.
// JS: holesail-server/index.js:put, holesail-client/index.js:ping, hyperdht/index.js:mutablePut, hyperdht/lib/persistent.js:signMutable
import { createHash } from 'node:crypto'
import { req, hex, fromHex, writeVectors } from './lib.mjs'

const HyperDHT = req('hyperdht')
const createTestnet = req('hyperdht/testnet')
const HolesailServer = req('holesail-server')
const HolesailClient = req('holesail-client')
const Persistent = req('hyperdht/lib/persistent')
const m = req('hyperdht/lib/messages')
const { NS } = req('hyperdht/lib/constants')
const c = req('compact-encoding')
const sodium = req('sodium-universal')
const z32 = req('z32')
const PKGS = ['holesail-server', 'holesail-client', 'hyperdht', 'compact-encoding', 'sodium-universal', 'z32']
const sha = (s) => createHash('sha256').update(s).digest()
const enc = (t, v) => hex(c.encode(t, v))
const b2b = (b) => { const o = Buffer.alloc(32); sodium.crypto_generichash(o, b); return o }
const guard = setTimeout(() => { console.error('live run timeout'); process.exit(1) }, 90000)

// ---------- (1) pure vectors ----------
const seeds = { abc: sha('abc'), rnd: fromHex('000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f') }
const values = [
  ['json tcp', '{"host":"127.0.0.1","port":3000}'],
  ['json udp', '{"host":"127.0.0.1","udp":true,"port":3000}'],
  ['json udp string false', '{"host":"127.0.0.1","udp":"false","port":3000}'],
  ['empty', ''],
  ['not json', 'not json'],
  ['utf8', '{"host":"χοστ","port":3000}'],
]
const seqs = [0, 1, 2, 127, 128, 252, 253, 65535, 65536, 4294967295, 4294967296]
const signing = []
for (const [name, seed] of Object.entries(seeds)) {
  const kp = HyperDHT.keyPair(seed)
  for (const [label, text] of values) for (const seq of seqs) {
    if (seq > 2 && label !== 'json tcp') continue // full seq enumeration — on one value
    const value = Buffer.from(text)
    const signature = Persistent.signMutable(seq, value, kp)
    signing.push({
      seed: name, seedHex: hex(seed), publicKey: hex(kp.publicKey), target: hex(b2b(kp.publicKey)), label, seq, valueHex: hex(value), valueText: text,
      signableEncodingHex: enc(m.mutableSignable, { seq, value }),
      signableHex: hex(NS.MUTABLE_PUT) + hex(b2b(c.encode(m.mutableSignable, { seq, value }))),
      signature: hex(signature),
      verify: Persistent.verifyMutable(signature, seq, value, kp.publicKey),
      putRequestHex: enc(m.mutablePutRequest, { publicKey: kp.publicKey, seq, value, signature }),
      getResponseHex: enc(m.mutableGetResponse, { seq, value, signature }),
      getRequestValueHex: enc(c.uint, seq),
    })
  }
}
// negative verifyMutable checks: the signature is valid only for the same (seq, value, publicKey)
const kp0 = HyperDHT.keyPair(seeds.abc), kp1 = HyperDHT.keyPair(seeds.rnd)
const v0 = Buffer.from(values[0][1]), sig0 = Persistent.signMutable(5, v0, kp0)
const bad = (label, seq, value, pk, sig) => ({ label, seq, valueHex: hex(value), publicKey: hex(pk), signature: hex(sig), verify: Persistent.verifyMutable(sig, seq, value, pk) })
const flip = Buffer.from(sig0); flip[0] ^= 1
const negative = [
  bad('valid', 5, v0, kp0.publicKey, sig0), bad('seq+1', 6, v0, kp0.publicKey, sig0), bad('value changed', 5, Buffer.from(values[1][1]), kp0.publicKey, sig0),
  bad('other publicKey', 5, v0, kp1.publicKey, sig0), bad('signature bit flipped', 5, v0, kp0.publicKey, flip),
]
const putTargetGet = { note: 'MUTABLE_GET request on put and on read: value = c.encode(c.uint, seq); on put always seq 0', putQueryValueHex: enc(c.uint, 0) }

// ---------- (2) live run on a testnet ----------
async function serverCase(label, { key, secure, udp, host = '127.0.0.1', port = 3000, seedHex }) {
  const testnet = await createTestnet(4) // own network per case: a stopped node of the previous case does not slow requests
  const dht = new HyperDHT({ bootstrap: testnet.bootstrap, host: '127.0.0.1', ephemeral: false, firewalled: false })
  const server = Object.create(HolesailServer.prototype) // without the constructor: it would create a HyperDHT with the public bootstrap
  Object.assign(server, { logger: { log() {} }, dht, stats: {}, server: null, keyPair: null, seed: null, state: null, connection: null, refreshInterval: null, activeConnections: new Map() })
  const seed = seedHex ?? hex(sha(key))
  const readerDht = testnet.createNode()
  const reader = Object.create(HolesailClient.prototype)
  reader.dht = readerDht
  reader.logger = { log() {} }
  const steps = []
  const read = async (pk) => {
    reader.publicKey = pk
    const r = await reader.get({ latest: true }) // real HolesailClient.get → mutableGet; returns {seq, value:string}
    const raw = await readerDht.mutableGet(pk, { latest: true }) // get() does not return the signature — take it from the same mutableGet
    if (!r !== !raw) throw new Error('get and mutableGet diverged: ' + JSON.stringify([r, raw && raw.seq]))
    return r && { seq: r.seq, valueText: r.value, signature: hex(raw.signature) }
  }
  const started = new Promise((res) => server.start({ port, host, seed, secure, udp }, res))
  await started
  const pub = server.keyPair.publicKey
  steps.push({ step: 'start', read: await read(pub) })
  const puts = [
    ['same value again', JSON.stringify({ host, udp, port })],
    ['changed port', JSON.stringify({ host, udp, port: port + 1 })],
    ['changed port again', JSON.stringify({ host, udp, port: port + 2 })],
    ['back to original', JSON.stringify({ host, udp, port })],
    ['same as previous', JSON.stringify({ host, udp, port })],
  ]
  for (const [what, data] of puts) {
    const seq = await server.put(data)
    steps.push({ step: 'put ' + what, putReturnedSeq: seq, putValueText: data, read: await read(pub) })
  }
  const out = { label, args: { host, port, udp: udp === undefined ? 'undefined' : udp, secure, seedHex: seed }, publicKey: hex(pub), target: hex(b2b(pub)), steps }
  // check against the pure signature: the signature from the network = signMutable(seq, value)
  for (const s of steps) if (s.read && hex(Persistent.signMutable(s.read.seq, Buffer.from(s.read.valueText), server.keyPair)) !== s.read.signature) throw new Error('signature from the network did not match signMutable: ' + label + ' ' + s.step)
  out.reader = { dht: readerDht }
  const pingKey = secure ? z32.encode(sha(key)) : z32.encode(pub) // as Holesail.lookup: private — z32(sha256(K)), public — z32(publicKey)
  out.ping = await HolesailClient.ping(pingKey, readerDht)
  delete out.reader
  await server.destroy()
  await readerDht.destroy()
  await testnet.destroy()
  return out
}
const live = []
live.push(await serverCase('private tcp key=abc', { key: 'abc', secure: true, udp: undefined }))
live.push(await serverCase('private udp', { key: 'abcdefghijklmnopqrstuvwxyz012345', secure: true, udp: true, port: 4000 }))
live.push(await serverCase('public tcp (seed = sha256(key))', { key: '000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f', secure: false, udp: undefined, port: 3001 }))
live.push(await serverCase('public udp string false', { key: 'κλωξ-κλωξ-κλωξ-κλωξ-κλωξ-κλωξ-κλωξ', secure: false, udp: 'false', port: 3002, host: 'localhost' }))
// lookup: no record and a non-JSON record
const testnet = await createTestnet(4)
const readerDht = testnet.createNode()
const nokey = z32.encode(HyperDHT.keyPair(sha('nothing-published')).publicKey)
const noRecord = await HolesailClient.ping(nokey, readerDht)
const kpBad = HyperDHT.keyPair(sha('not-json-record'))
await readerDht.mutablePut(kpBad, Buffer.from('not json'), { seq: 0 })
const notJson = await HolesailClient.ping(z32.encode(kpBad.publicKey), readerDht)
const kpUtf = HyperDHT.keyPair(sha('utf8-record'))
await readerDht.mutablePut(kpUtf, Buffer.from('{"host":"χοστ","port":3000}'), { seq: 0 })
const utf8 = await HolesailClient.ping(z32.encode(kpUtf.publicKey), readerDht)
await readerDht.destroy()
await testnet.destroy()
clearTimeout(guard)

const p1 = writeVectors('holesail', 'dht-record-signing', { note: 'Mutable record signature and encodings. signable = NS.MUTABLE_PUT(32) ‖ blake2b-256(signableEncoding); signableEncoding = c.uint(seq) ‖ c.buffer(value). target = blake2b-256(publicKey). putRequest = fixed32 publicKey, uint seq, buffer value, fixed64 signature; getResponse = uint seq, buffer value, fixed64 signature. Ed25519 is deterministic: the signature repeats byte for byte.', nsMutablePutHex: hex(NS.MUTABLE_PUT), putTargetGet, signing, negative }, PKGS)
const p2 = writeVectors('holesail', 'dht-record-live', { note: 'Real HolesailServer.start/put, HolesailClient.get/ping on a local testnet (a new network of 4 nodes on 127.0.0.1 for each case). seq and value — step by step; the signature is checked against signMutable. ping — the result of HolesailClient.ping(argKey), as in Holesail.lookup.', cases: live, lookupExtra: { noRecord, notJsonRecord: notJson, utf8Record: utf8, notJsonPublicKey: hex(kpBad.publicKey) } }, PKGS)
console.log(p1, signing.length, negative.length, p2, live.length)
