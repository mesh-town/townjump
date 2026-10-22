/**
 * dht-rpc interop testnet: a bootstrapper plus two persistent nodes, all on 127.0.0.1.
 *
 * The nodes use `ephemeral: false, firewalled: false, host: '127.0.0.1'`, otherwise a JS node
 * sends from its client socket without an id and never lands in a Rust routing table.
 *
 * Protocol:
 *   stdout line 1: { "ready": true, "port": <bootstrapper port> }
 *   stdin  { "cmd": "table", "id": n }    → { "id": n, "nodes": [{ "id": "<hex>", "host": "...", "port": n }] }
 *            (union of the routing tables of all three nodes)
 *   stdin  { "cmd": "shutdown", "id": n } → destroy all nodes and exit
 *   stdin close → same as shutdown
 */

const DHT = require('dht-rpc')
const readline = require('readline')

async function freePort () {
  const probe = require('dgram').createSocket('udp4')
  await new Promise((resolve, reject) => {
    probe.bind(0, '127.0.0.1', (err) => (err ? reject(err) : resolve()))
  })
  const port = probe.address().port
  probe.close()
  return port
}

function tableUnion (nodes) {
  const seen = new Map()
  for (const dht of nodes) {
    for (const n of dht.table.toArray()) {
      const id = Buffer.from(n.id).toString('hex')
      seen.set(id, { id, host: n.host, port: n.port })
    }
  }
  return [...seen.values()]
}

async function main () {
  const bootstrapper = DHT.bootstrapper(await freePort(), '127.0.0.1')
  await bootstrapper.ready()
  const port = bootstrapper.address().port
  const bootstrap = ['127.0.0.1:' + port]

  const nodes = [bootstrapper]
  for (let i = 0; i < 2; i++) {
    const node = new DHT({ bootstrap, ephemeral: false, firewalled: false, host: '127.0.0.1' })
    await node.fullyBootstrapped()
    nodes.push(node)
  }

  process.stdout.write(JSON.stringify({ ready: true, port }) + '\n')

  let closing = false
  const shutdown = async () => {
    if (closing) return
    closing = true
    await Promise.all(nodes.map((n) => n.destroy()))
    process.exit(0)
  }

  const rl = readline.createInterface({ input: process.stdin })
  rl.on('line', (line) => {
    let msg
    try { msg = JSON.parse(line) } catch { return }
    if (msg.cmd === 'table') {
      process.stdout.write(JSON.stringify({ id: msg.id, nodes: tableUnion(nodes) }) + '\n')
    } else if (msg.cmd === 'shutdown') {
      shutdown()
    }
  })
  rl.on('close', shutdown)
}

main().catch((err) => {
  process.stderr.write(err.stack + '\n')
  process.exit(1)
})
