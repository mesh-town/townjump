#!/usr/bin/env node
// TCP server: @hyperswarm/secret-stream responder set up like a hyperdht connection (keepAlive 5000)
// plus an idle timeout (IDLE_TIMEOUT_MS, default 7000): the peer must keep the stream alive with keepalives.
// Protocol: "ping" -> "pong"; after an idle pause "still here" -> "alive", then end.
// An empty data event (keepalive that was not filtered) or "Stream timed out" fails the run.

const net = require('net')
const SecretStream = require('@hyperswarm/secret-stream')

const PORT = parseInt(process.env.PORT || '0', 10)
const IDLE_TIMEOUT_MS = parseInt(process.env.IDLE_TIMEOUT_MS || '7000', 10)

const server = net.createServer((socket) => {
  const ss = new SecretStream(false, socket, { keepAlive: 5000 })
  ss.setTimeout(IDLE_TIMEOUT_MS)

  ss.on('data', (data) => {
    const msg = data.toString()
    if (msg === 'ping') return ss.write(Buffer.from('pong'))
    if (msg === 'still here') {
      ss.write(Buffer.from('alive'))
      return ss.end()
    }
    process.stderr.write(`Unexpected message (${data.length} bytes): "${msg}"\n`)
    process.exit(1)
  })

  ss.on('error', (err) => {
    process.stderr.write(`SecretStream error: ${err.message}\n`)
  })

  ss.on('close', () => {
    server.close()
  })
})

server.listen(PORT, '127.0.0.1', () => {
  process.stdout.write(`LISTENING:${server.address().port}\n`)
})

server.on('error', (err) => {
  process.stderr.write(`Server error: ${err.message}\n`)
  process.exit(1)
})
