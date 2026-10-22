// Harness scenarios. Each runs with any pair {client, server}; `phrase: true` — the private server gets
// `--key <phrase>`, otherwise the key is random and is taken from the banner. The client always connects by the `hs://…` link.
import { randomPayload, tcpEcho, tcpNoTunnel, udpEcho, udpTruncated } from './checks.mjs'
import { serverPublicKeyZ32 } from './keys.mjs'
import { randomHex } from './world.mjs'

const MIB = 1 << 20

const tcpOnce = (size) => async (w) => {
  const client = await w.startClient(w.url)
  await tcpEcho(client.port, randomPayload(size))
  w.expectIncoming(1)
}

const udpOnce = async (w) => {
  const client = await w.startClient(w.url)
  await udpEcho(client.port)
  w.expectIncoming(1)
}

const udpTruncate = async (w) => {
  const client = await w.startClient(w.url)
  await udpTruncated(client.port)
  w.expectIncoming(1)
}

// Two connections in a row through one client (one identity): the second handshake must not break because of the first.
const tcpTwoInARow = async (w) => {
  const client = await w.startClient(w.url)
  await tcpEcho(client.port, randomPayload(1000))
  await tcpEcho(client.port, randomPayload(MIB))
  w.expectIncoming(2)
}

// Rejection: a connection without data and without an "Incoming…" line on the server; then a control — the own key passes
// (otherwise "no line" could mean that the server is dead or the log is different).
const rejected = (foreignUrl) => async (w) => {
  const foreign = await w.startClient(foreignUrl(w))
  await tcpNoTunnel(foreign.port, w.ctx.negTimeoutMs)
  w.expectIncoming(0)
  const own = await w.startClient(w.url)
  await tcpEcho(own.port, randomPayload(1000))
  w.expectIncoming(1)
}

const wrongKey = (w) => `hs://s000other-${randomHex(16)}` // K' ≠ K: a different public key, there is no record or server under it
const publicClientOfPrivate = (w) => `hs://0000${serverPublicKeyZ32(w.privateKey)}` // the same server key, but a random client identity

export const SCENARIOS = [
  { id: 'tcp-private', proto: 'tcp', mode: 'private', phrase: true, run: tcpOnce(MIB) },
  { id: 'tcp-public', proto: 'tcp', mode: 'public', run: tcpOnce(MIB) },
  { id: 'udp-private', proto: 'udp', mode: 'private', run: udpOnce },
  { id: 'udp-public', proto: 'udp', mode: 'public', run: udpOnce },
  { id: 'udp-truncate', proto: 'udp', mode: 'private', run: udpTruncate },
  { id: 'tcp-private-two-conn', proto: 'tcp', mode: 'private', run: tcpTwoInARow },
  { id: 'tcp-private-big', proto: 'tcp', mode: 'private', phrase: true, run: tcpOnce(8 * MIB) },
  { id: 'fw2-wrong-key', proto: 'tcp', mode: 'private', run: rejected(wrongKey) },
  { id: 'fw3-public-client', proto: 'tcp', mode: 'private', phrase: true, run: rejected(publicClientOfPrivate) },
  // Only on request (`--only nat`), not in the default matrix: it is flaky even on JS↔JS because of holepunch on loopback.
  // The JS client emulates a client behind NAT (remoteAddress() = null); the Rust client has no such emulation — for it this is a plain tcp-private.
  { id: 'tcp-private-nat', proto: 'tcp', mode: 'private', extra: true, clientEnv: { INTEROP_JS_FIREWALL: 'unknown' }, run: tcpOnce(MIB) }
]
