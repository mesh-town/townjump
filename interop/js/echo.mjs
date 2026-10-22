// Echo service for the harness: TCP (allowHalfOpen, answers even after a half-close) and UDP on 127.0.0.1.
// As a module: `startEcho()` → { tcpPort, udpPort, close() }. As a program: `node interop/js/echo.mjs [tcpPort] [udpPort]`
// prints `ECHO tcp=<port> udp=<port>` and runs until SIGTERM.
import dgram from 'node:dgram'
import net from 'node:net'
import { fileURLToPath } from 'node:url'

const HOST = '127.0.0.1'

function listenTcp(port) {
  const server = net.createServer({ allowHalfOpen: true }, (socket) => {
    socket.on('error', () => {})
    socket.pipe(socket) // on client EOF pipe calls end() itself: the echo is finished and closed
  })
  return new Promise((resolve, reject) => {
    server.once('error', reject)
    server.listen(port, HOST, () => resolve(server))
  })
}

function listenUdp(port) {
  const socket = dgram.createSocket('udp4')
  socket.on('message', (msg, rinfo) => socket.send(msg, rinfo.port, rinfo.address))
  return new Promise((resolve, reject) => {
    socket.once('error', reject)
    socket.bind(port, HOST, () => resolve(socket))
  })
}

export async function startEcho({ tcpPort = 0, udpPort = 0 } = {}) {
  const tcp = await listenTcp(tcpPort)
  const udp = await listenUdp(udpPort)
  return {
    tcpPort: tcp.address().port,
    udpPort: udp.address().port,
    close: () => new Promise((resolve) => { udp.close(); tcp.close(() => resolve()); tcp.closeAllConnections?.() })
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const [tcpPort, udpPort] = process.argv.slice(2).map(Number)
  const echo = await startEcho({ tcpPort, udpPort })
  console.log(`ECHO tcp=${echo.tcpPort} udp=${echo.udpPort}`)
  process.on('SIGTERM', () => echo.close().then(() => process.exit(0)))
  process.on('SIGINT', () => echo.close().then(() => process.exit(0)))
}
