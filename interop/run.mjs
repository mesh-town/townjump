// Holesail compatibility harness: matrix {client} × {server} × scenarios on a local testnet, output "scenario → ok/fail".
// Both sides are real CLIs (JS 2.4.4 or a Rust binary). How to run and what it checks — interop/README.md.
import fs from 'node:fs'
import path from 'node:path'
import { HELP, parseArgs } from './lib/args.mjs'
import { JS_NODE_MODULES, ROOT_DIR, resolveRustBin } from './lib/impl.mjs'
import { SCENARIOS } from './lib/scenarios.mjs'
import { startTestnet } from './lib/testnet.mjs'
import { World } from './lib/world.mjs'

const SCENARIO_LIMIT_MS = 180_000

function withTimeout(promise, ms, label) {
  let timer
  const limit = new Promise((_, reject) => { timer = setTimeout(() => reject(new Error(`${label}: scenario timed out after ${ms / 1000} s`)), ms) })
  return Promise.race([promise, limit]).finally(() => clearTimeout(timer))
}

async function runOne(ctx, pair, scenario) {
  const started = Date.now()
  const world = new World(ctx, pair, scenario)
  try {
    await withTimeout((async () => { await world.startServer(); await scenario.run(world) })(), SCENARIO_LIMIT_MS, scenario.id)
    return { status: 'ok', ms: Date.now() - started }
  } catch (e) {
    return { status: 'FAIL', ms: Date.now() - started, error: `${e.message}\n${world.diagnostics()}` }
  } finally {
    await world.cleanup()
  }
}

function selectScenarios(only) {
  if (only.length === 0) return SCENARIOS.filter((s) => !s.extra)
  return SCENARIOS.filter((s) => only.some((part) => (s.extra ? s.id === part : s.id.includes(part))))
}

function pairsOf(opts) {
  return opts.client.flatMap((client) => opts.server.map((server) => ({ client, server })))
}

const line = (status, pair, id, ms, note = '') => `${status.padEnd(5)} ${`${pair.client}→${pair.server}`.padEnd(9)} ${id.padEnd(22)} ${ms === null ? '' : `${(ms / 1000).toFixed(1)}s`.padStart(6)} ${note}`.trimEnd()

async function runMatrix(opts, ctx, scenarios) {
  const totals = { ok: 0, FAIL: 0, skip: 0 }
  for (const pair of pairsOf(opts)) {
    if ((pair.client === 'rust' || pair.server === 'rust') && !ctx.rustBin) {
      totals.skip += scenarios.length
      console.log(line('skip', pair, `(${scenarios.length} scenarios)`, null, 'no Rust binary: --rust-bin or TOWNJUMP_BIN'))
      continue
    }
    for (const scenario of scenarios) {
      const r = await runOne(ctx, pair, scenario)
      totals[r.status]++
      console.log(line(r.status, pair, scenario.id, r.ms))
      if (r.error) console.log(r.error.split('\n').map((l) => `      ${l}`).join('\n'))
    }
  }
  return totals
}

async function main() {
  const opts = parseArgs(process.argv.slice(2))
  if (opts.help) return console.log(HELP)
  const scenarios = selectScenarios(opts.only)
  if (opts.list) return SCENARIOS.forEach((s) => console.log(`${s.id}\t${s.proto}\t${s.mode}${s.extra ? '\tonly via --only' : ''}`))
  const outDir = path.join(ROOT_DIR, '.tmp/interop-out')
  fs.rmSync(outDir, { recursive: true, force: true })
  fs.mkdirSync(outDir, { recursive: true })
  const rustBin = resolveRustBin(opts.rustBin)
  if (opts.rustBin && !rustBin) throw new Error(`Rust binary not found: ${opts.rustBin}`)
  const testnet = await startTestnet(opts.nodes, path.join(outDir, 'testnet.log'))
  console.log(`harness: client→server; testnet ${testnet.bootstrap} (${opts.nodes} nodes); JS ${JS_NODE_MODULES}; Rust ${rustBin ?? 'none'}; logs ${outDir}`)
  try {
    const totals = await runMatrix(opts, { bootstrap: testnet.bootstrap, rustBin, outDir, negTimeoutMs: opts.negTimeoutMs, settleMs: opts.settleMs, serverEnv: opts.mutate ? { INTEROP_JS_MUTATE: opts.mutate } : {} }, scenarios)
    console.log(`total: ${totals.ok} ok, ${totals.FAIL} fail, ${totals.skip} skip`)
    process.exitCode = totals.FAIL > 0 || (opts.strict && totals.skip > 0) ? 1 : 0
  } finally {
    await testnet.proc.stop()
  }
}

main().catch((e) => { console.error(`startup error: ${e.message}`); process.exit(2) })
