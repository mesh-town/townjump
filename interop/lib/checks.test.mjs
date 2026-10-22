// Self-check of `udpTruncated`: on a fake "proxy" it accepts truncation to 2048 and rejects the rest.
import assert from 'node:assert/strict'
import dgram from 'node:dgram'
import test from 'node:test'
import { udpTruncated } from './checks.mjs'

// Fake proxy: answers every datagram with what `reply(msg)` returns (a list of datagrams).
async function fakeProxy(reply) {
  const socket = dgram.createSocket('udp4')
  socket.on('message', (msg, rinfo) => reply(msg).forEach((part) => socket.send(part, rinfo.port, rinfo.address)))
  await new Promise((resolve) => socket.bind(0, '127.0.0.1', resolve))
  return { port: socket.address().port, close: () => socket.close() }
}

const run = async (reply) => {
  const proxy = await fakeProxy(reply)
  try {
    await udpTruncated(proxy.port, 4000, 200, 5000)
  } finally {
    proxy.close()
  }
}

test('truncation to 2048 and a working tunnel — accepted', () => run((m) => [m.subarray(0, 2048)]))

test('whole 4000 datagram — rejected', async () => {
  await assert.rejects(run((m) => [m]), /got 4000, not 2048/)
})

test('remainder as a separate datagram — rejected', async () => {
  await assert.rejects(run((m) => (m.length > 100 ? [m.subarray(0, 2048), m.subarray(2048)] : [m])), /after truncation got 1952/)
})

test('truncated not from the start — rejected', async () => {
  await assert.rejects(run((m) => [m.subarray(1952)]), /did not match/)
})
