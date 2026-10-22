// Rust-CLIENT relay scenarios of probe-relay.mjs: a `probe connect --relay-through` against JS/Rust relays and servers,
// and the relay -> direct upgrade seen from the Rust side. Registered by probe-relay.mjs: rsScenarios(helpers).
// Success marks: client prints `PATH relayed` (and, in the upgrade, later `PATH direct`) and the 1 MiB echo matches.

import crypto from 'node:crypto'

const lines = (proc, re) => proc.text.split('\n').filter((l) => re.test(l))
const portOf = (line, field) => Number(new RegExp(`${field}=\\S*:(\\d+)`).exec(line)?.[1])

export function rsScenarios(h) {
  const { keyOf, seed, bin, BYTES, EXPECTED, sleep, within, once, pausePunching } = h
  const hex = (b) => Buffer.from(b).toString('hex')

  function rustClient(ctx, relayPk, serverPk, extra) {
    const argv = ['connect', '--server', hex(serverPk), '--seed', hex(seed(0x12)), '--relay-through', hex(relayPk), ...extra]
    return ctx.rust('rust-client', bin('PROBE_BIN', 'probe'), argv)
  }

  async function finish(proc) {
    await proc.waitFor(/ECHO \d+ \w+/, 40_000)
    const [, total, digest] = /ECHO (\d+) (\w+)/.exec(proc.text)
    if (Number(total) !== BYTES) throw new Error(`echo: received ${total} bytes instead of ${BYTES}`)
    if (digest !== EXPECTED) throw new Error(`echo: sha256 ${digest} ≠ ${EXPECTED}`)
  }

  // Both ends must have used the relay: JS relay paired two streams, Rust relay matched a pairing and opened two streams.
  async function relayWasUsed(ctx, relay) {
    if (relay.counters) {
      if (relay.counters.streams < 2) throw new Error('echo ok but the JS relay paired no streams (the connection was direct)')
      return
    }
    await sleep(500)
    const last = lines(relay.proc, /^STATS/).pop() ?? 'no STATS'
    ctx.note(`rust relay: ${last}`)
    if (!/pairings_matched=[1-9]/.test(last) || !/streams_opened=([2-9]|\d\d)/.test(last)) throw new Error(`echo ok but the Rust relay did not carry it: ${last}`)
  }

  const noDirect = (proc) => {
    if (lines(proc, /^PATH direct/).length > 0) throw new Error(`client reports a direct path: ${lines(proc, /^PATH direct/)[0]}`)
  }

  // Rust client -> (JS|Rust relay) -> (JS|Rust server); nobody punches, so the relay is the only path.
  const relayed = (relayRust, serverRust) => async (ctx) => {
    const relay = relayRust ? await h.startRustRelay(ctx) : await h.startJsRelay(ctx)
    const rustServer = serverRust ? await h.startRustServer(ctx, ['--no-holepunch']) : null
    if (!serverRust) await h.startJsServer(ctx)
    const punching = pausePunching(ctx.nodes)
    ctx.onCleanup = punching.restore
    const client = rustClient(ctx, relay.pk, keyOf(serverRust ? 0x21 : 0x11).publicKey, ['--no-holepunch'])
    await finish(client)
    if (!/^PATH relayed /m.test(client.text)) throw new Error('client never reported PATH relayed')
    noDirect(client)
    if (rustServer) {
      await rustServer.waitFor(/ECHOED \w+ 1048576/, 5000)
      if (!/^PATH relayed /m.test(rustServer.text)) throw new Error('server never reported PATH relayed')
    }
    await relayWasUsed(ctx, relay)
  }

  // Rust client + JS server announcing the relay; the client delays punching by 2 s and pauses the echo in the middle.
  const upgrade = (relayRust) => async (ctx) => {
    const relay = relayRust ? await h.startRustRelay(ctx) : await h.startJsRelay(ctx)
    const server = await h.startJsServer(ctx, { relayThrough: relay.pk })
    const client = rustClient(ctx, relay.pk, keyOf(0x11).publicKey, ['--punch-delay-ms', '2000', '--echo-pause-ms', '8000'])
    await within(client.waitFor(/^PATH relayed /m, 20_000), 21_000, 'PATH relayed')
    await within(client.waitFor(/^PATH direct /m, 25_000), 26_000, 'PATH direct (no upgrade)')
    // The server's ports are read now: after the echo the stream is closed and reports 0.
    const raw = server.rawStream()
    const ports = { local: raw.localPort, remote: raw.remotePort }
    const relayed = lines(client, /^PATH relayed /)[0]
    const direct = lines(client, /^PATH direct /)[0]
    await finish(client)
    if (portOf(direct, 'remote') !== ports.local) throw new Error(`${direct}: client remote ≠ server local port ${ports.local}`)
    if (portOf(direct, 'local') !== ports.remote) throw new Error(`${direct}: client local ≠ server remote port ${ports.remote}`)
    if (portOf(relayed, 'remote') === ports.local) throw new Error('client started on the direct path')
    if (relayRust) await closedRustRelay(ctx, relay)
    else await Promise.all(relay.sockets.map((s) => (s.destroyed ? null : within(once(s, 'close'), 10_000, 'relay socket to close'))))
    if (!relayRust && relay.sockets.length < 2) throw new Error(`relay saw ${relay.sockets.length} transport sockets, need the client and the server`)
    ctx.note(`upgrade: ${relayed} -> ${direct}`)
  }

  async function closedRustRelay(ctx, relay) {
    await sleep(1000)
    const last = lines(relay.proc, /^STATS/).pop() ?? 'no STATS'
    ctx.note(`rust relay after upgrade: ${last}`)
    if (!/pairings_active=0/.test(last) || !/streams_closed=([2-9]|\d\d)/.test(last)) throw new Error(`Rust relay not released after the upgrade: ${last}`)
  }

  // ---- Rust SERVER scenarios (JS client, J1: the client is the non-initiator of the relay pairing) ----
  const statsOf = (relay) => lines(relay.proc, /^STATS/).pop() ?? 'no STATS'
  const payload = () => Buffer.from(Array.from({ length: BYTES }, (_, i) => i % 251))

  async function rustServerOn(ctx, relay, extra) {
    return h.startRustServer(ctx, ['--relay-through', hex(relay.pk), ...extra])
  }

  function jsClient(ctx, node, opts) {
    const socket = node.connect(keyOf(0x21).publicKey, { keyPair: keyOf(0x12), localConnection: false, fastOpen: false, ...opts })
    socket.on('error', (e) => ctx.note(`client error: ${e.message}`))
    return socket
  }

  // Whole 1 MiB through a half-close; resolves with the byte count and sha256 of the echo.
  function echoAll(socket) {
    return new Promise((resolve, reject) => {
      const hash = crypto.createHash('sha256')
      let total = 0
      socket.on('data', (d) => { hash.update(d); total += d.length })
      socket.on('end', () => resolve({ total, digest: hash.digest('hex') }))
      socket.on('open', () => socket.end(payload()))
      socket.on('close', () => reject(new Error('connection closed before the echo ended')))
    })
  }

  function checkAll({ total, digest }) {
    if (total !== BYTES || digest !== EXPECTED) throw new Error(`echo: ${total} bytes, sha256 ${digest}`)
  }

  // Rust server announces the relay, nobody punches: JS client -> relay -> Rust server.
  const rsServerRelayed = (relayRust) => async (ctx) => {
    const relay = relayRust ? await h.startRustRelay(ctx) : await h.startJsRelay(ctx)
    const server = await rustServerOn(ctx, relay, ['--no-holepunch'])
    const punching = pausePunching(ctx.nodes)
    ctx.onCleanup = punching.restore
    checkAll(await echoAll(jsClient(ctx, ctx.leaf(), { holepunch: () => false })))
    await server.waitFor(/ECHOED \w+ 1048576/, 5000)
    if (!/^PATH relayed /m.test(server.text)) throw new Error('server never reported PATH relayed')
    if (lines(server, /^PATH direct/).length > 0) throw new Error(`server reports a direct path: ${lines(server, /^PATH direct/)[0]}`)
    await relayWasUsed(ctx, relay)
  }

  // JS client holds punching until half of the data is echoed over the relay; then the Rust server must switch to PATH direct.
  const rsServerUpgrade = (relayRust) => async (ctx) => {
    const relay = relayRust ? await h.startRustRelay(ctx) : await h.startJsRelay(ctx)
    // The server delays its own punching: otherwise its probes reach the paused client and the direct path can win the race.
    const server = await rustServerOn(ctx, relay, ['--punch-delay-ms', '3000'])
    const clientNode = ctx.leaf()
    const punching = pausePunching([clientNode])
    ctx.onCleanup = punching.restore
    const socket = jsClient(ctx, clientNode)
    await within(once(socket, 'open'), 20_000, 'client open')
    const raw = socket.rawStream
    const hash = crypto.createHash('sha256')
    let total = 0
    const half = BYTES / 2
    const data = payload()
    const firstHalf = new Promise((resolve) => socket.on('data', (d) => { hash.update(d); total += d.length; if (total >= half) resolve() }))
    socket.write(data.subarray(0, half))
    await within(firstHalf, 20_000, 'echo of the 1st half over the relay')
    await within(server.waitFor(/^PATH relayed /m, 5000), 6000, 'server PATH relayed')
    if (lines(server, /^PATH direct/).length > 0) throw new Error('server was direct before punching resumed')
    punching.resume()
    await within(server.waitFor(/^PATH direct /m, 25_000), 26_000, 'server PATH direct (no upgrade)')
    const direct = lines(server, /^PATH direct /)[0]
    const ended = once(socket, 'end')
    await sleep(500)
    if (portOf(direct, 'local') !== raw.remotePort) throw new Error(`${direct}: server local ≠ client remote port ${raw.remotePort}`)
    if (portOf(direct, 'remote') !== raw.localPort) throw new Error(`${direct}: server remote ≠ client local port ${raw.localPort}`)
    socket.end(data.subarray(half))
    await within(ended, 20_000, 'echo end after upgrade')
    checkAll({ total, digest: hash.digest('hex') })
    if (relayRust) await closedRustRelay(ctx, relay)
    else await Promise.all(relay.sockets.map((s) => (s.destroyed ? null : within(once(s, 'close'), 10_000, 'relay socket to close'))))
    ctx.note(`rust server: ${lines(server, /^PATH /).join(' | ')}`)
  }

  // D14: JS peers over the Rust relay; after both close, the relay holds no pairing, no session, and the counters moved.
  async function d14RustRelayJsPeers(ctx) {
    const relay = await h.startRustRelay(ctx)
    await h.startJsServer(ctx, { relayThrough: relay.pk })
    const punching = pausePunching(ctx.nodes)
    ctx.onCleanup = punching.restore
    const socket = ctx.leaf().connect(keyOf(0x11).publicKey, {
      keyPair: keyOf(0x12), relayThrough: relay.pk, holepunch: () => false, localConnection: false, fastOpen: false
    })
    socket.on('error', () => {})
    checkAll(await echoAll(socket))
    socket.destroy()
    const released = (l) => /pairings_active=0/.test(l) && /sessions_active=0/.test(l) && /streams_closed=([2-9]|\d\d)/.test(l) && /pairings_matched=[1-9]/.test(l)
    const until = Date.now() + 10_000
    while (!released(statsOf(relay)) && Date.now() < until) await sleep(250)
    ctx.note(`rust relay at the end: ${statsOf(relay)}`)
    if (!released(statsOf(relay))) throw new Error(`Rust relay kept state after the peers closed: ${statsOf(relay)}`)
  }

  return {
    'rs-server-relay-through-js-client': rsServerRelayed(true),
    'rs-server-relay-through-js-relay': rsServerRelayed(false),
    'rs-server-upgrade': rsServerUpgrade(false),
    'rs-server-upgrade-rust-relay': rsServerUpgrade(true),
    'd14-rust-relay-js-peers': d14RustRelayJsPeers,
    'rs-client-js-relay-js-server': relayed(false, false),
    'rs-client-rust-relay-js-server': relayed(true, false),
    'rs-client-rust-relay-rust-server': relayed(true, true),
    'rs-upgrade': upgrade(false),
    'rs-upgrade-rust-relay': upgrade(true)
  }
}
