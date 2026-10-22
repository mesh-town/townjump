// Data exchange through the client's local port: TCP echo with a half-close, UDP echo, "no tunnel".
import crypto from 'node:crypto'
import dgram from 'node:dgram'
import net from 'node:net'

const HOST = '127.0.0.1'
const sha = (buf) => crypto.createHash('sha256').update(buf).digest('hex').slice(0, 16)

// Sends the payload, waits for the whole echo and only then closes its half (FIN) — the server must answer EOF.
// The harness does not send FIN right after writing (`socket.end(payload)`): with such a close the JS reference loses in-flight data
// sometimes, and Rust has to be checked against a defined JS behavior.
export function tcpEcho(port, payload, timeoutMs = 60_000) {
  return new Promise((resolve, reject) => {
    const chunks = []
    let received = 0
    let closed = false
    const socket = net.connect({ port, host: HOST, allowHalfOpen: true })
    const timer = setTimeout(() => finish(new Error(`TCP :${port}: no EOF in ${timeoutMs} ms (received ${received} of ${payload.length} bytes)`)), timeoutMs)
    const finish = (err) => {
      clearTimeout(timer)
      socket.destroy()
      const got = Buffer.concat(chunks)
      if (err) return reject(err)
      if (got.equals(payload)) return resolve()
      reject(new Error(`TCP :${port}: response differs from request: ${got.length} bytes (sha ${sha(got)}) instead of ${payload.length} (sha ${sha(payload)})`))
    }
    socket.on('data', (d) => {
      chunks.push(d)
      received += d.length
      if (received < payload.length || closed) return
      closed = true
      socket.end()
    })
    socket.on('end', () => finish())
    socket.on('error', (e) => finish(new Error(`TCP :${port}: ${e.message}`)))
    socket.on('connect', () => socket.write(payload))
  })
}

// The connection is accepted locally, but there is no tunnel: no data must arrive, the connection must close by itself.
export function tcpNoTunnel(port, timeoutMs) {
  return new Promise((resolve, reject) => {
    const socket = net.connect({ port, host: HOST })
    const timer = setTimeout(() => done(new Error(`TCP :${port}: neither close nor data in ${timeoutMs} ms`)), timeoutMs)
    const done = (err) => {
      clearTimeout(timer)
      socket.destroy()
      if (err) reject(err)
      else resolve()
    }
    socket.on('data', (d) => done(new Error(`TCP :${port}: foreign key got ${d.length} bytes of response — there is a tunnel, and there must not be`)))
    socket.on('close', () => done())
    socket.on('error', () => {}) // ECONNRESET counts as a close, the result comes from 'close'
    socket.on('connect', () => socket.write('ping\n'))
  })
}

// The sender sends numbered datagrams of varying length; the echo of each must come back intact.
export function udpEcho(port, count = 20, timeoutMs = 30_000) {
  const sizes = [5, 100, 1200, 1472, 2048]
  const sent = Array.from({ length: count }, (_, i) => {
    const body = crypto.randomBytes(sizes[i % sizes.length])
    body.writeUInt32BE(i, 0)
    return body
  })
  return new Promise((resolve, reject) => {
    const socket = dgram.createSocket('udp4')
    const back = new Map()
    const timer = setTimeout(() => done(new Error(`UDP :${port}: returned ${back.size} of ${count} datagrams in ${timeoutMs} ms`)), timeoutMs)
    const done = (err) => {
      clearTimeout(timer)
      socket.close()
      if (err) return reject(err)
      const bad = sent.findIndex((b, i) => !back.get(i)?.equals(b))
      if (bad >= 0) return reject(new Error(`UDP :${port}: datagram ${bad} did not match: ${describeDiff(sent[bad], back.get(bad))}`))
      resolve()
    }
    socket.on('message', (msg) => {
      back.set(msg.readUInt32BE(0), msg)
      if (back.size === count) done()
    })
    socket.on('error', (e) => done(new Error(`UDP :${port}: ${e.message}`)))
    socket.bind(0, HOST, () => sent.forEach((b) => socket.send(b, port, HOST)))
  })
}

// A datagram over 2048 bytes: holesail accepts at most 2048 on the local socket, the remainder is lost (JS: bare-dgram
// over udx-native). The application gets exactly one response of the first 2048 bytes, without a second fragment; then a control
// datagram passes — truncation does not break the tunnel.
export const UDP_LIMIT = 2048

export function udpTruncated(port, size = 4000, quietMs = 1500, timeoutMs = 30_000) {
  const big = crypto.randomBytes(size)
  const control = crypto.randomBytes(100)
  return new Promise((resolve, reject) => {
    const socket = dgram.createSocket('udp4')
    const got = []
    const timer = setTimeout(() => done(new Error(`UDP :${port}: in ${timeoutMs} ms got ${got.length} responses of 2`)), timeoutMs)
    let quiet = null
    const done = (err) => {
      clearTimeout(timer)
      clearTimeout(quiet)
      socket.close()
      err ? reject(err) : resolve()
    }
    const onBig = (msg) => {
      if (msg.length !== UDP_LIMIT) return done(new Error(`UDP :${port}: for ${size} bytes got ${msg.length}, not ${UDP_LIMIT}`))
      if (!msg.equals(big.subarray(0, UDP_LIMIT))) return done(new Error(`UDP :${port}: response did not match the first ${UDP_LIMIT} bytes (sha ${sha(msg)} instead of ${sha(big.subarray(0, UDP_LIMIT))})`))
      quiet = setTimeout(() => socket.send(control, port, HOST), quietMs) // wait whether the remainder arrives as a separate datagram
    }
    const onControl = (msg) => (msg.equals(control) ? done() : done(new Error(`UDP :${port}: after truncation got ${msg.length} bytes instead of the control ${control.length}`)))
    socket.on('message', (msg) => {
      got.push(msg.length)
      got.length === 1 ? onBig(msg) : onControl(msg)
    })
    socket.on('error', (e) => done(new Error(`UDP :${port}: ${e.message}`)))
    socket.bind(0, HOST, () => socket.send(big, port, HOST))
  })
}

export const randomPayload = (n) => crypto.randomBytes(n)

function describeDiff(sent, got) {
  if (!got) return 'no response'
  const at = sent.findIndex((b, i) => got[i] !== b)
  return `sent ${sent.length} bytes, returned ${got.length}, first difference at byte ${at}`
}
