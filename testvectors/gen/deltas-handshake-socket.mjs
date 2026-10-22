// D11: which UDP socket the server answers a client handshake from, for a client on the same network (127.0.0.1) — hyperdht 6.34.0 (new) against 6.30.0 (old).
// We wrap Server.prototype._onpeerhandshake (lib/server.js: returns { socket, noise }) and compare the address of the announced socket with the socket of the server DHT node.
// References: hyperdht 6.33.0 (69bfcf0) — replyFromPuncher; with the same external client address (ourRemoteAddr) the regular server socket remains. Output: testvectors/deltas/handshake-socket.json
import { createRequire } from 'node:module'
import { ROOT, NM, writeVectors } from './lib.mjs'
const OLD_NM = ROOT + '/.tmp/peeroxide-node-theirs/node_modules'
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
async function run(nm) {
  const r = createRequire(nm + '/hyperdht/')
  const Server = r('./lib/server.js'), createTestnet = r('./testnet.js'), DHT = r('./')
  const seen = []
  const orig = Server.prototype._onpeerhandshake
  Server.prototype._onpeerhandshake = async function (...a) {
    const res = await orig.apply(this, a)
    if (res) seen.push({ hasPuncherSocket: !!res.socket, samePortAsServerDhtSocket: res.socket ? res.socket.address().port === this.dht.io.serverSocket?.address().port || res.socket.address().port === this.dht.io.clientSocket?.address().port : null, socketNull: res.socket == null })
    return res
  }
  const testnet = await createTestnet(4, { host: '127.0.0.1' })
  const serverNode = testnet.nodes[1], clientNode = new DHT({ bootstrap: testnet.bootstrap, host: '127.0.0.1' })
  await sleep(300)
  const kp = DHT.keyPair(Buffer.alloc(32, 7))
  const server = serverNode.createServer((s) => { s.on('error', () => {}); s.end() }); await server.listen(kp)
  await sleep(300)
  const socket = clientNode.connect(kp.publicKey); socket.on('error', () => {})
  let opened = false; socket.on('open', () => { opened = true })
  for (let i = 0; !opened && i < 100; i++) await sleep(50)
  await sleep(300)
  Server.prototype._onpeerhandshake = orig
  const out = { version: r('hyperdht/package.json').version, connectionOpened: opened, repliesSeenAtLeastOne: seen.length > 0, everyReplyAdvertisedNoPuncherSocket: seen.every((x) => x.socketNull) }
  socket.destroy(); await server.close(); await clientNode.destroy(); await testnet.destroy()
  return out
}
writeVectors('deltas', 'handshake-socket', { note: 'Client and server on one host 127.0.0.1 (the same external address). everyReplyAdvertisedNoPuncherSocket=true — in all handshake replies the server did not announce the holepuncher socket (the reply left from the regular server socket). Ports are not written. On the loopback testnet both versions are the same: the branch «client on the same network + private addresses» (replacing the reply from the holepuncher socket with the server socket) is not distinguished by the 127.0.0.1 testnet.', new: await run(NM), old: await run(OLD_NM) }, ['hyperdht'])
process.exit(0)
