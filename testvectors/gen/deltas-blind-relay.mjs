// D14: blind-relay 1.6.1 (new, reference) against 1.4.0 (old) — what is closed on the relay server when one of the clients sends unpair (or disappears) with an active pair.
// Test rig without a network: a BlindRelay.Server, two "sessions" on a streamx-Duplex pair, BlindRelay.Client clients on the other end; the server creates the pair streams with real udx-native (createStream) but connects them nowhere.
// Output: testvectors/deltas/blind-relay.json. References: blind-relay/index.js (_onunpair, BlindRelayLink.destroyPair), commit 0bfb303.
import { createRequire } from 'node:module'
import path from 'node:path'
import { ROOT, NM } from './lib.mjs'
import { writeVectors } from './lib.mjs'
import { VERSIONS } from './udx-rig.mjs'
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
const DIRS = { new: path.join(NM, 'blind-relay'), old: path.join(ROOT, '.tmp/peeroxide-node-theirs/node_modules/blind-relay') }
const token = Buffer.alloc(32, 0x42)

async function scenario(ver, who) {
  const dir = DIRS[ver]
  const req = createRequire(path.join(dir, 'index.js'))
  const Relay = req(dir), { Duplex } = req('streamx'), UDX = createRequire(import.meta.url)(VERSIONS[ver])
  const udx = new UDX(), made = []
  const server = new Relay.Server({ createStream: (opts) => { const s = udx.createStream(100 + made.length, opts); s.on('error', () => {}); made.push(s); return s } })
  const link = () => { let a, b; a = new Duplex({ write(d, cb) { b.push(d); cb() } }); b = new Duplex({ write(d, cb) { a.push(d); cb() } }); a.on('error', () => {}); b.on('error', () => {}); return [a, b] }
  const [cA, sA] = link(), [cB, sB] = link()
  const sessA = server.accept(sA), sessB = server.accept(sB)
  sessA.on('error', () => {}); sessB.on('error', () => {})
  const clA = new Relay.Client(cA), clB = new Relay.Client(cB)
  clA.on('error', () => {}); clB.on('error', () => {})
  const rA = clA.pair(true, token, { id: 11 }), rB = clB.pair(false, token, { id: 12 })
  rA.on('error', () => {}).on('data', () => {}); rB.on('error', () => {}).on('data', () => {})
  const t0 = Date.now(); while (made.length < 2 && Date.now() - t0 < 3000) await sleep(5)
  await sleep(100)
  const before = { streamsCreatedByServer: made.length, destroyed: made.map((s) => !!s.destroyed) }
  if (who === 'initiatorUnpair') clA.unpair(token)
  else if (who === 'responderUnpair') clB.unpair(token)
  else if (who === 'initiatorSessionGone') { cA.destroy(); sA.destroy() }
  await sleep(300)
  const after = { destroyed: made.map((s) => !!s.destroyed) }
  const st = server.stats // 1.4.0 has no stats counters
  const serverStats = st ? { pairingsMatched: st.pairings.matched, pairingsCancelled: st.pairings.cancelled, streamsOpened: st.streams.opened, streamsClosed: st.streams.closed, sessionsClosed: st.sessions.closed } : 'no stats in this version'
  return { before, after, serverStats }
}

const out = {}
for (const ver of Object.keys(DIRS)) {
  out[ver] = {}
  for (const who of ['initiatorUnpair', 'responderUnpair', 'initiatorSessionGone']) {
    try { out[ver][who] = await scenario(ver, who) } catch (e) { out[ver][who] = { error: String(e.stack || e.message).split('\n').slice(0, 3).join(' | ') } }
  }
}
if (process.argv[2] === '--print') { console.log(JSON.stringify(out, null, 1)); process.exit(0) }
console.log(writeVectors('deltas', 'blind-relay', { note: 'Blind-relay server without a network: new = 1.6.1, old = 1.4.0. Token is 32 bytes of 0x42. Order of streams in the destroyed arrays: [responder, initiator] (the server creates them by links[+isInitiator]); initiatorSessionGone = the initiator connection to the server dropped.', results: out }, []))
process.exit(0)
