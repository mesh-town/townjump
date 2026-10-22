// Reference key-derivation vectors of holesail 2.4.4 (no network: HyperDHT.keyPair from a seed only). Writes keyderivation-vectors.json
import { createRequire } from 'node:module'
import { createHash } from 'node:crypto'
import { writeFileSync } from 'node:fs'
const require = createRequire(new URL('../../.tmp/npm-holesail/node_modules/holesail/', import.meta.url))
const HyperDHT = require('hyperdht')
const z32 = require('z32')
const sha = (s) => createHash('sha256').update(s).digest()
const Ks = ['abc', 'abcdefghijklmnopqrstuvwxyz012345', '000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f', 'κλωξ-κλωξ-κλωξ-κλωξ-κλωξ-κλωξ-κλωξ', 'a b', '']
const priv = Ks.filter((k) => k).map((K) => {
  const seed = sha(K.toString())                       // server: hex(sha256(utf8(K))) -> Buffer.from(hex,'hex')
  const kp = HyperDHT.keyPair(seed)                     // server and client derive the key pair the same way
  return { K, sha256Seed: seed.toString('hex'), publicKey: kp.publicKey.toString('hex'), clientSeedZ32: z32.encode(seed), publicKeyZ32: z32.encode(kp.publicKey), url: 'hs://s000' + K, lookupArgKey: z32.encode(seed) }
})
const pubSeeds = ['000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f'].map((hex) => {
  const kp = HyperDHT.keyPair(Buffer.from(hex, 'hex'))  // public mode without --key: seed = 32 random bytes as is
  return { seedHex: hex, publicKey: kp.publicKey.toString('hex'), publicKeyZ32: z32.encode(kp.publicKey), url: 'hs://0000' + z32.encode(kp.publicKey) }
})
const numeric = [123456, '0123456789012345678901234567890123456789'].map((v) => ({ argvText: String(v), note: 'minimist turns a number-like string into a Number; the key becomes String(Number)', asKey: String(Number(v)) }))
writeFileSync(new URL('./keyderivation-vectors.json', import.meta.url), JSON.stringify({ private: priv, publicNoKey: pubSeeds, numericMangling: numeric }, null, 1))
console.log(JSON.stringify(priv.map((p) => [p.K.slice(0, 12), p.sha256Seed.slice(0, 16), p.publicKey.slice(0, 16), p.clientSeedZ32.slice(0, 12)])))
console.log(JSON.stringify(pubSeeds), JSON.stringify(numeric))
