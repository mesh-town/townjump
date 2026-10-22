// How the real CLI of each side is launched. Both accept the same argv and TOWNJUMP_BOOTSTRAP.
// JS: node -r js/bootstrap-env.cjs <reference holesail.mjs>; Rust: the binary directly or through the TOWNJUMP_RUN_WRAPPER wrapper
// The place where Rust is plugged in is resolveRustBin.
import fs from 'node:fs'
import path from 'node:path'
import { createRequire } from 'node:module'

const require = createRequire(import.meta.url)
const { ROOT, CLI, NM } = require('../js/nm.cjs')
const BOOTSTRAP_ENV = path.join(ROOT, 'interop/js/bootstrap-env.cjs')
// Launch wrapper — from TOWNJUMP_RUN_WRAPPER; without it the binary runs directly.
const RUN_WRAPPER = process.env.TOWNJUMP_RUN_WRAPPER || null

export const JS_CLI_DIR = path.dirname(path.dirname(path.dirname(CLI)))
export const JS_NODE_MODULES = NM
export const ROOT_DIR = ROOT

// The command name is the `name` of the first [[bin]] in crates/townjump/Cargo.toml (the only place where it is set).
export function binName(root) {
  const file = path.join(root, 'crates/townjump/Cargo.toml')
  const m = fs.readFileSync(file, 'utf8').match(/^\[\[bin\]\][^\[]*?^name\s*=\s*"([^"]+)"/m)
  if (!m) throw new Error(`no [[bin]] with a name in ${file}`)
  return m[1]
}

export function resolveRustBin(explicit) {
  const candidates = explicit
    ? [explicit]
    : [process.env.CARGO_TARGET_DIR, path.join(ROOT, 'target')]
        .filter(Boolean)
        .flatMap((dir) => ['release', 'debug'].map((profile) => path.join(dir, profile, binName(ROOT))))
  const found = candidates.find((p) => fs.existsSync(p))
  return found ? path.resolve(found) : null
}

// impl = 'js' | 'rust'; rustBin — a path or null. Returns { cmd, args } without env (env is shared, see childEnv).
export function commandFor(impl, argv, rustBin) {
  if (impl === 'js') return { cmd: process.execPath, args: ['-r', BOOTSTRAP_ENV, CLI, ...argv] }
  if (RUN_WRAPPER) return { cmd: '/bin/sh', args: [RUN_WRAPPER, rustBin, ...argv] }
  return { cmd: rustBin, args: argv }
}

export function childEnv(bootstrap, extra = {}) {
  return { ...process.env, TOWNJUMP_BOOTSTRAP: bootstrap, NO_COLOR: '1', ...extra }
}
