// Shared generator helpers: paths from the file location, require from the holesail tree, JSON writing. Neighbor: all.mjs
import { createRequire } from 'node:module'
import { mkdirSync, writeFileSync, readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import path from 'node:path'

export const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')
export const NM = path.join(ROOT, '.tmp/npm-holesail/node_modules')
export const VECTORS = path.join(ROOT, 'testvectors')
// require from the holesail package root: finds hoisted dependencies (hyperdht, dht-rpc, z32, ...) of exactly the versions pinned by the lockfile
export const req = createRequire(NM + '/holesail/')

export const version = (pkg) => JSON.parse(readFileSync(path.join(NM, pkg, 'package.json'), 'utf8')).version
export const hex = (b) => Buffer.from(b).toString('hex')
export const fromHex = (h) => Buffer.from(h, 'hex')

// Write testvectors/<layer>/<name>.json. Only deterministic: no time, randomness or absolute paths.
export function writeVectors(layer, name, body, pkgs = []) {
  const meta = { generator: 'testvectors/gen/' + layer + '.' + name.split('.')[0] + ' (see all.mjs)', node: process.version.replace(/^v(\d+).*/, '$1.x'), versions: Object.fromEntries(pkgs.map((p) => [p, version(p)])) }
  const dir = path.join(VECTORS, layer)
  mkdirSync(dir, { recursive: true })
  const text = JSON.stringify({ _meta: meta, ...body }, (k, v) => (typeof v === 'bigint' ? v.toString() : v), 1) + '\n'
  writeFileSync(path.join(dir, name + '.json'), text)
  return path.join(layer, name + '.json')
}

export function count(x) {
  return Array.isArray(x) ? x.length : Object.values(x).reduce((n, v) => n + (Array.isArray(v) ? v.length : 0), 0)
}
