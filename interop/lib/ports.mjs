// Free ports on 127.0.0.1: every scenario takes new ones so that the previous run (TIME_WAIT, an orphan) does not interfere.
import dgram from 'node:dgram'
import net from 'node:net'

const HOST = '127.0.0.1'

export function freeTcpPort() {
  return new Promise((resolve, reject) => {
    const server = net.createServer()
    server.once('error', reject)
    server.listen(0, HOST, () => {
      const { port } = server.address()
      server.close(() => resolve(port))
    })
  })
}

export function freeUdpPort() {
  return new Promise((resolve, reject) => {
    const socket = dgram.createSocket('udp4')
    socket.once('error', reject)
    socket.bind(0, HOST, () => {
      const { port } = socket.address()
      socket.close(() => resolve(port))
    })
  })
}
