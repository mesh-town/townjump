/**
 * HyperDHT interop test peer for server unannounce.
 *
 * Starts a local testnet (1 bootstrap + 4 DHT nodes) and a client `peer`,
 * then exposes a command interface over stdin/stdout so the Rust
 * integration test can drive it.
 *
 * Protocol:
 *   stdout line 1: { "ready": true, "port": <bootstrap_port> }
 *
 *   stdin commands (one JSON per line):
 *     { "cmd": "find_peer", "public_key": "<hex32>", "id": <n> }
 *     { "cmd": "shutdown" }
 *
 *   stdout replies (one JSON per line):
 *     { "id": <n>, "ok": true, "found": <bool> }   // found: some node answered
 *                                                  // FIND_PEER with that public key
 *     { "id": <n>, "ok": false, "error": "<msg>" } // any error
 */

'use strict'

const DHT = require('hyperdht')
const b4a = require('b4a')
const readline = require('readline')

async function main () {
  const bootstrap = new DHT({
    ephemeral: false,
    firewalled: false,
    host: '127.0.0.1',
    port: 0,
    bootstrap: []
  })
  await bootstrap.fullyBootstrapped()
  const bsPort = bootstrap.address().port
  const bsAddr = [{ host: '127.0.0.1', port: bsPort }]

  const nodes = []
  for (let i = 0; i < 4; i++) {
    const node = new DHT({ ephemeral: false, firewalled: false, host: '127.0.0.1', bootstrap: bsAddr })
    await node.fullyBootstrapped()
    nodes.push(node)
  }

  const peer = new DHT({ ephemeral: true, host: '127.0.0.1', bootstrap: bsAddr })
  await peer.ready()

  process.stdout.write(JSON.stringify({ ready: true, port: bsPort }) + '\n')

  const rl = readline.createInterface({ input: process.stdin, crlfDelay: Infinity })

  rl.on('line', async (line) => {
    let req
    try {
      req = JSON.parse(line.trim())
    } catch (e) {
      return
    }

    const { cmd, id } = req

    try {
      if (cmd === 'find_peer') {
        const publicKey = b4a.from(req.public_key, 'hex')
        let found = false
        for await (const data of peer.findPeer(publicKey)) {
          if (data.peer && b4a.equals(data.peer.publicKey, publicKey)) found = true
        }
        process.stdout.write(JSON.stringify({ id, ok: true, found }) + '\n')

      } else if (cmd === 'shutdown') {
        await cleanup()
        process.exit(0)
      }
    } catch (err) {
      process.stdout.write(JSON.stringify({ id, ok: false, error: err.message }) + '\n')
    }
  })

  rl.on('close', async () => {
    await cleanup()
    process.exit(0)
  })

  async function cleanup () {
    try { await peer.destroy() } catch (_) {}
    for (const node of nodes.reverse()) {
      try { await node.destroy() } catch (_) {}
    }
    try { await bootstrap.destroy() } catch (_) {}
  }
}

main().catch((err) => {
  process.stderr.write(err.stack + '\n')
  process.exit(1)
})
