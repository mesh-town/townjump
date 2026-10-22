// D1: external command 10 (PLUGIN) and unknown 11 against a testnet node on 127.0.0.1 — new (hyperdht 6.34.0) and old (6.30.0).
// JS: hyperdht/index.js:_onrequest, hyperdht/lib/persistent.js:onplugin, dht-rpc/index.js:_onrequest. Output: testvectors/deltas/plugin.json
import { createRequire } from 'node:module'
import { ROOT, NM, writeVectors } from './lib.mjs'
const OLD_NM = ROOT + '/.tmp/peeroxide-node-theirs/node_modules'
async function run(nm) {
  const r = createRequire(nm + '/hyperdht/')
  const createTestnet = r('./testnet.js')
  const testnet = await createTestnet(4, { host: '127.0.0.1' })
  // nodes[0] — the first testnet node: it has no NAT samples, it stays without id and not persistent; nodes[1..] become persistent by neighbor replies and get an id (dht-rpc: _updateNetworkState)
  const first = testnet.nodes[0], server = testnet.nodes[1], client = testnet.nodes[2]
  for (let i = 0; server.id === null || client.id === null; i++) { if (i > 100) throw new Error('nodes did not become persistent within 5 s'); await new Promise((res) => setTimeout(res, 50)) }
  // plugin "x" v1 on a persistent node: in 6.34.0 persistent.onplugin calls onrequest; in 6.30.0 this branch does not exist
  const hasPluginsApi = !!server.plugins // 6.30.0 has no plugins map
  if (hasPluginsApi) server.plugins.set('x', { version: 1, onrequest(plugreq, req) { req.reply(Buffer.from('pong')) }, onpersistent() {}, destroy() {} })
  const target = Buffer.alloc(32, 9)
  const ask = async (node, label, { internal = false, command, target = null, value = null }) => {
    const to = { host: '127.0.0.1', port: node.address().port }
    const q = client.io.createRequest({ id: null, host: to.host, port: to.port }, null, internal, command, target, value, null, 0)
    q.retries = 0; q.timeout = 700
    try { const res = await client._requestToPromise(q, {}); return { label, internal, command, outcome: 'reply', error: res.error, value: res.value ? res.value.toString('hex') : null } }
    catch (e) { return { label, internal, command, outcome: 'error', code: e.code, errorNumber: e.errno ?? null } }
  }
  const batch = async (node) => [
    await ask(node, 'control: PING (0), internal', { internal: true, command: 0 }),
    await ask(node, 'control: FIND_PEER (2), external, 32-byte target', { command: 2, target }),
    await ask(node, 'PLUGIN (10), external, value null', { command: 10 }),
    await ask(node, 'PLUGIN (10), external, pluginRequest "x" v1 cmd 2 (the plugin is registered on the node only for withId)', { command: 10, value: Buffer.from('0178010200', 'hex') }),
    await ask(node, 'PLUGIN (10), external, pluginRequest "y" v1 cmd 2 (the plugin is not registered)', { command: 10, value: Buffer.from('0179010200', 'hex') }),
    await ask(node, 'PLUGIN (10), internal (internal flag: dht-rpc does not pass it to hyperdht)', { internal: true, command: 10 }),
    await ask(node, 'unknown external (11)', { command: 11 }),
  ]
  const state = (n) => ({ hasPluginsApi, persistent: n._persistent !== null, hasId: n.id !== null, ephemeral: n.ephemeral })
  const withoutId = { ...state(first), results: await batch(first) }
  const withId = { ...state(server), results: await batch(server) }
  await testnet.destroy()
  return { version: r('hyperdht/package.json').version, withoutId, withId }
}
writeVectors('deltas', 'plugin', { note: 'Requests (external hyperdht commands; PING and one PLUGIN row carry the internal flag for comparison) to two testnet nodes on 127.0.0.1: withoutId — the first node (no id, not persistent, receiving internal commands is closed), withId — a persistent node with an id obtained normally (dht-rpc: _updateNetworkState from neighbor samples). outcome is reply (error 0/code) or error (exception code: REQUEST_TIMEOUT — silence). Request timeout 700 ms without retransmissions. Keys new/old: hyperdht 6.34.0 / 6.30.0.', new: await run(NM), old: await run(OLD_NM) }, ['hyperdht', 'dht-rpc'])
process.exit(0)
