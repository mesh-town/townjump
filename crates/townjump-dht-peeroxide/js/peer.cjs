// Live-test JS peer for the adapter (hyperdht 6.34.0): echo server or client, events line by line on stdout.
// Usage: node js/peer.cjs serve <bootstrap host:port[,…]> <seedHex64> [timeoutMs]
//         node js/peer.cjs dial  <bootstrap host:port[,…]> <pubKeyHex64> [timeoutMs]
// Stdout: `<Date.now() ms> <EVENT> [data]`: READY <pubHex> (serve), CONNECTION (serve) | OPEN (dial), DATA <utf8>,
// ERROR <code or message>, CLOSE. Every DATA is echoed back. Stdin: `send <text>` (to the last socket), `quit`;
// closing stdin exits. Env PEER_RELAY_THROUGH=<hex64> (dial): connect with `relayThrough`; such a peer does not echo. See also: src/js_peer_support_tests.rs (parent), interop/js/nm.cjs (hyperdht path).
const readline = require('readline')
const { req } = require('../../../interop/js/nm.cjs')
const DHT = req('hyperdht')

const [mode, bootstrapArg, keyHex, timeoutArg] = process.argv.slice(2)
if (!['serve', 'dial'].includes(mode) || !bootstrapArg || !/^[0-9a-f]{64}$/.test(keyHex || '')) {
  console.error('usage: peer.cjs serve|dial <bootstrap> <hex64> [timeoutMs]')
  process.exit(2)
}
const timeoutMs = timeoutArg ? Number(timeoutArg) : 0
const bootstrap = bootstrapArg.split(',').map((s) => s.trim()).filter(Boolean)

const relayHex = process.env.PEER_RELAY_THROUGH
const log = (event, data) => console.log(`${Date.now()} ${event}${data === undefined ? '' : ` ${data}`}`)
const dht = new DHT({ bootstrap })
let last = null

function attach(socket) {
  last = socket
  if (timeoutMs > 0) socket.setTimeout(timeoutMs)
  socket.on('data', (chunk) => {
    log('DATA', chunk.toString('utf8'))
    if (!relayHex) socket.write(chunk)
  })
  socket.on('error', (err) => log('ERROR', err.code || err.message))
  socket.on('close', () => log('CLOSE'))
}

async function main() {
  if (mode === 'serve') {
    const kp = DHT.keyPair(Buffer.from(keyHex, 'hex'))
    const server = dht.createServer()
    server.on('connection', (socket) => {
      log('CONNECTION')
      attach(socket)
    })
    await server.listen(kp)
    log('READY', kp.publicKey.toString('hex'))
  } else {
    const socket = dht.connect(Buffer.from(keyHex, 'hex'), relayHex ? { relayThrough: Buffer.from(relayHex, 'hex') } : {})
    attach(socket)
    socket.on('open', () => log('OPEN'))
  }
}

async function quit(code) {
  await dht.destroy().catch(() => {})
  process.exit(code)
}

readline.createInterface({ input: process.stdin }).on('line', (line) => {
  if (line === 'quit') return quit(0)
  if (line.startsWith('send ') && last) last.write(Buffer.from(line.slice(5), 'utf8'))
}).on('close', () => quit(0))

main().catch((err) => {
  log('ERROR', err.code || err.message)
  quit(1)
})
