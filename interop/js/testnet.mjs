// Local hyperdht testnet: bootstrap and N ordinary nodes on 127.0.0.1 in one node process.
// Run: node interop/js/testnet.mjs [N] [--exit-on-stdin-eof]   (N defaults to 4).
// Contract: the first stdout line is `TOWNJUMP_BOOTSTRAP=127.0.0.1:<port>` (the value fits an environment variable
// of the Rust and JS binaries as is). Lives until SIGTERM/SIGINT; with the flag — also until stdin closes
// (a parent with a pipe on stdin leaves no orphans). The same process is used by JsTestnet::start in townjump-testkit.
import { createRequire } from 'node:module'

const require = createRequire(import.meta.url)
const { req } = require('./nm.cjs')
const createTestnet = req('hyperdht/testnet')

const args = process.argv.slice(2)
const size = Number(args.find((a) => /^\d+$/.test(a)) ?? 4)
const testnet = await createTestnet(size)
const bootstrap = testnet.bootstrap.map((b) => `${b.host}:${b.port}`).join(',')

let closing = false
async function shutdown() {
  if (closing) return
  closing = true
  await testnet.destroy()
  process.exit(0)
}

process.on('SIGTERM', shutdown)
process.on('SIGINT', shutdown)
if (args.includes('--exit-on-stdin-eof')) {
  process.stdin.on('end', shutdown)
  process.stdin.resume()
}
console.log(`TOWNJUMP_BOOTSTRAP=${bootstrap}`)
