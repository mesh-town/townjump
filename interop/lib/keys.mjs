// Public key of a private server from key K: seed = sha256(K), the pair is HyperDHT.keyPair(seed) (spec connection-string.md).
// Needed by the scenario "public client on a private server": the server banner prints K, not this key.
import crypto from 'node:crypto'
import { createRequire } from 'node:module'

const require = createRequire(import.meta.url)
const { req } = require('../js/nm.cjs')

export function serverPublicKeyZ32(privateKey) {
  const seed = crypto.createHash('sha256').update(privateKey).digest()
  return req('z32').encode(req('hyperdht').keyPair(seed).publicKey)
}
