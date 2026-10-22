// Holesail layer: urlParser and key derivation (server, client) through the real Holesail/HolesailServer/HolesailClient classes without a network.
// JS: holesail/src/index.js, holesail-server/index.js, holesail-client/index.js. Output: testvectors/holesail/{url-parser,key-derivation}.json
import { createRequire } from 'node:module'
import { readFileSync } from 'node:fs'
import path from 'node:path'
import { req, hex, writeVectors, NM, ROOT } from './lib.mjs'

const Holesail = req('holesail')
const HolesailServer = req('holesail-server')
const HolesailClient = req('holesail-client')
const HyperDHT = req('hyperdht')
const z32 = req('z32')
const libKeys = req('hyper-cmd-lib-keys')
const libKeysServer = createRequire(NM + '/holesail-server/')('hyper-cmd-lib-keys') // nested 0.2.0, used by generateKeyPair
const PKGS = ['holesail', 'holesail-server', 'holesail-client', 'hyperdht', 'z32', 'hyper-cmd-lib-keys']

// ---------- urlParser ----------
const seedI = (i) => Buffer.alloc(32, i)
const bareByIndex = (want) => { // public z32 keys from seeds 0x00..: some with 's' at index 5, others without
  const out = []
  for (let i = 0; i < 400 && out.length < 3; i++) {
    const k = z32.encode(HyperDHT.keyPair(seedI(i)).publicKey)
    if ((k[5] === 's') === want) out.push(k)
  }
  return out
}
const inputs = [
  undefined, null, '', 0, 123456, '0', false, true,
  'hs://s000abc', 'hs://0000abc', 'hs://x000abc', 'hs://sXYZabc', 'hs://s000', 'hs://0000', 'hs://s0000', 'hs://', 'hs://s00', 'hs://s000K',
  'hs://s000' + '00'.repeat(32), 'hs://0000' + z32.encode(HyperDHT.keyPair(seedI(0)).publicKey),
  'HS://s000abc', ' hs://s000abc', 'hs:/s000abc', 'hs//s000abc', 'https://s000abc', 'hs://S000abc',
  'hs://s000hs://s000x', 'abc', 'abcde', 'abcdes', 'abcdesX', 'abcdefs', 'abcdef',
  'hs://κλωξ-κλωξ', 'hs://s000κλωξ', 'κλωξ-κλωξ-κλωξ', 'κλωξsκλωξ', 'κλωξ-κλωξ-s', '😀😀😀abc', 'hs://😀000abc', 'hs://s😀abc', 'ab😀s', 'abcd😀s0',
  ...bareByIndex(true), ...bareByIndex(false),
]
const enc = (v) => (v === undefined ? { type: 'undefined' } : v === null ? { type: 'null' } : { type: typeof v, value: v })
const urlParser = inputs.map((v) => { const r = Holesail.urlParser(v); return { input: enc(v), key: r.key, secure: r.secure === undefined ? null : r.secure } })
const urlPath = writeVectors('holesail', 'url-parser', {
  note: 'Holesail.urlParser(input) as is. secure: null means undefined («not set»), the method never returns false. input.type: as typeof in JS, undefined and null are marked separately.',
  vectors: urlParser,
}, PKGS)

// ---------- key derivation ----------
const quiet = { log() {} }
const derive = (h, role) => role === 'server' ? deriveServer(h) : deriveClient(h)
function deriveServer(h) {
  const fake = { logger: quiet, secure: h.secure === true } // HolesailServer.start: this.secure = args.secure === true
  const keyPair = HolesailServer.prototype.generateKeyPair.call(fake, h.seed)
  const key = Object.getOwnPropertyDescriptor(HolesailServer.prototype, 'key').get.call(fake)
  h.dht = { info: { type: 'server', key } } // Holesail.info takes only key from dht; the rest is not needed
  const info = h.info
  return { seedHex: h.seed ?? null, generatedSeedHex: hex(fake.seed), publicKey: hex(keyPair.publicKey), secretKey: hex(keyPair.secretKey), serverKey: key, infoKey: info.key, url: info.url }
}
function deriveClient(h) {
  const c = new HolesailClient({ key: h.seed, secure: h.secure, logger: quiet })
  const out = { seedZ32OrKey: h.seed, secureFlag: c.secure, publicKey: hex(c.publicKey), keyPair: c.keyPair ? { publicKey: hex(c.keyPair.publicKey), secretKey: hex(c.keyPair.secretKey) } : null }
  c.dht.destroy()
  return out
}
const rnd = '000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f'
const cases = []
const add = (role, key, secure) => cases.push({ role, key, secure })
const K = ['abc', 'abcdefghijklmnopqrstuvwxyz012345', rnd, 'κλωξ-κλωξ-κλωξ-κλωξ-κλωξ-κλωξ-κλωξ', 'hs://s000abc', 'hs://0000abc', 'hs://s000' + rnd, 'hs://0000' + z32.encode(HyperDHT.keyPair(Buffer.from(rnd, 'hex')).publicKey), 123456, '0123456789012345678901234567890123456789']
for (const key of K) for (const secure of [undefined, true, false, 'false']) { add('server', key, secure); add('client', key, secure) }
for (const secure of [undefined, true, false, 'false']) add('server', undefined, secure) // without --key: the secret or seed is random
const results = []
for (const cs of cases) {
  const forced = { randomHex: rnd }
  libKeys.randomBytes = libKeysServer.randomBytes = () => Buffer.from(forced.randomHex, 'hex') // the only source of randomness in holesail
  const opts = { [cs.role]: true, port: 3000, host: '127.0.0.1', key: cs.key, secure: cs.secure }
  const input = { role: cs.role, key: enc(cs.key), secure: cs.secure === undefined ? { type: 'undefined' } : { type: typeof cs.secure, value: cs.secure }, randomBytesHex: cs.key === undefined ? rnd : null }
  try {
    const h = new Holesail(opts)
    const holesail = { key: h.key === undefined ? null : h.key, seed: h.seed === undefined ? null : h.seed, secure: h.secure === undefined ? null : h.secure }
    results.push({ input, holesail, result: derive(h, cs.role) })
  } catch (e) {
    results.push({ input, error: { name: e.name, message: e.message } })
  }
}
// check against the already captured keyderivation-vectors.json: the same K → the same seed, public key and z32 client seed
const old = JSON.parse(readFileSync(path.join(ROOT, 'testvectors/holesail-cli/keyderivation-vectors.json'), 'utf8'))
for (const o of old.private) {
  const s = results.find((r) => r.input.role === 'server' && r.input.key.value === o.K && r.input.secure.type === 'undefined')
  const cl = results.find((r) => r.input.role === 'client' && r.input.key.value === o.K && r.input.secure.value === true)
  if (!s || !cl) continue
  if (s.result.seedHex !== o.sha256Seed || s.result.publicKey !== o.publicKey || cl.holesail.seed !== o.clientSeedZ32) throw new Error('mismatch with holesail-cli/keyderivation-vectors.json for K=' + o.K)
}
const keyPath = writeVectors('holesail', 'key-derivation', {
  note: 'Real new Holesail(opts) + HolesailServer.generateKeyPair + HolesailClient (constructor) without a network. randomBytes is replaced: always rnd. Server: seedHex — h.seed (hex sha256 K or null), generatedSeedHex — the seed the pair was built with (random without a secret), serverKey — HolesailServer.key, infoKey/url — Holesail.info. Client: seedZ32OrKey — h.seed, publicKey — HolesailClient.publicKey. Errors are as is (name, message). For K without a URL wrapper the same is in holesail-cli/keyderivation-vectors.json (checked by the script).',
  randomBytesHex: rnd,
  vectors: results,
}, PKGS)
console.log(urlPath, urlParser.length, keyPath, results.length, 'errors:', results.filter((r) => r.error).length)
