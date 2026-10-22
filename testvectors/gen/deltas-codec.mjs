// Version difference, codecs (D1 D12 D13): NS_PLUGIN and pluginRequest, the c.uint / c.buffer table, protomux batch frames — on the new and old version.
// JS: hyperdht/lib/constants.js:NS.PLUGIN, hyperdht/lib/messages.js:pluginRequest, compact-encoding/index.js, protomux/index.js:_onbatch. Output: testvectors/deltas/codec.json
import { createRequire } from 'node:module'
import path from 'node:path'
import { ROOT, NM, req, hex, writeVectors } from './lib.mjs'

const OLD = path.join(ROOT, '.tmp/peeroxide-node-theirs/node_modules')
const roots = { new: NM, old: OLD }
const rq = (ver, pkg) => createRequire(path.join(roots[ver], pkg) + '/')(roots[ver] === NM ? req.resolve(pkg) : path.join(roots[ver], pkg))
const load = (ver, pkg, from) => createRequire(path.join(roots[ver], from ?? pkg) + '/')(pkg)

// ---- D1: NS_PLUGIN, pluginRequest, NS prefixes ----
const consts = req('hyperdht/lib/constants.js')
const m = req('hyperdht/lib/messages.js')
const c = req('compact-encoding')
const crypto = req('hypercore-crypto')
const pluginRequest = [['value null', null], ['value empty Buffer', Buffer.alloc(0)], ['value aabb', Buffer.from('aabb', 'hex')], ['value undefined (key absent)', undefined]].map(([label, value]) => {
  const msg = { plugin: 'x', version: 1, command: 2 }
  if (value !== undefined) msg.value = value
  const b = c.encode(m.pluginRequest, msg)
  const d = c.decode(m.pluginRequest, b)
  return { label, plugin: 'x', version: 1, command: 2, valueHex: value === undefined ? 'absent' : value === null ? 'null' : hex(value), encodedHex: hex(b), decoded: { plugin: d.plugin, version: d.version, command: d.command, value: d.value === null ? 'null' : hex(d.value) } }
})
const nsIds = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]
const namespaces = crypto.namespace('hyperswarm/dht', nsIds).map((b, i) => ({ id: nsIds[i], hex: hex(b) }))
const d1 = { commands: consts.COMMANDS, nsPlugin: hex(consts.NS.PLUGIN), namespaces, pluginRequest }

// ---- D12: c.uint, c.buffer on 3.5.2 and 2.19.2 ----
const tryDec = (fn) => { try { return { ok: fn() } } catch (e) { return { error: e.message } } }
const tryEnc = (fn) => { try { return { ok: hex(fn()) } } catch (e) { return { error: e.message } } }
function d12(ver) {
  const cc = load(ver, 'compact-encoding')
  const uints = [0, 0xfc, 0xfd, 0xffff, 0x10000, 0xffffffff, 2 ** 32, 2 ** 53 - 1, 2 ** 53].map((v) => ({ value: String(v), encoded: tryEnc(() => cc.encode(cc.uint, v)) }))
  const decodeUints = ['00', 'fc', 'fdfd00', 'fdffff', 'fe00000100', 'feffffffff', 'ff0000000001000000', 'ffffffffffffff1f00', 'ff0000000000002000', 'ffffffffffffffff'].map((h) => ({ hex: h, decoded: tryDec(() => String(cc.decode(cc.uint, Buffer.from(h, 'hex')))) }))
  const bufferDecode = ['00', '03aabbcc', '0300', '03aa'].map((h) => ({ hex: h, decoded: tryDec(() => { const r = cc.decode(cc.buffer, Buffer.from(h, 'hex')); return r === null ? 'null' : hex(r) }) }))
  const bufferEncode = [['null', null], ['empty', Buffer.alloc(0)], ['aabb', Buffer.from('aabb', 'hex')]].map(([label, v]) => ({ label, encoded: tryEnc(() => cc.encode(cc.buffer, v)) }))
  const rawBufferEmpty = tryEnc(() => cc.encode(cc.raw.buffer, Buffer.alloc(0)))
  return { version: JSON.parse(JSON.stringify(load(ver, 'compact-encoding/package.json'))).version, uints, decodeUints, bufferDecode, bufferEncode, rawBufferEmpty }
}

// ---- D13: protomux, a batch with a length greater than the remainder ----
import { Duplex } from 'node:stream'
async function d13(ver) {
  const Protomux = load(ver, 'protomux')
  const cc = load(ver, 'compact-encoding', 'protomux')
  const frames = [
    ['plain message ch1 "abc"', '010003616263'],
    ['batch one exact (len 5)', '000001050003616263'],
    ['batch two "abc","def"', '000001050003616263050003646566'],
    ['batch two "R","R" (reply is batch)', '0000010300015203000152'],
    ['batch len 0x14 > rest', '000001140003616263'],
    ['batch len 0x14 > rest, string longer than data', '000001140009616263'],
    ['batch len 0 then switch channel, empty rest', '0000010001'],
    ['batch empty', '000001'],
  ]
  const results = []
  for (const [label, frameHex] of frames) {
    const written = []
    const stream = new Duplex({ read() {}, write(d, _e, cb) { written.push(Buffer.from(d).toString('hex')); cb() } })
    let closed = null, err = null
    stream.on('error', (e) => { err = e.message })
    const mux = new Protomux(stream)
    const ch = mux.createChannel({ protocol: 'p', onclose() { closed = 'channel closed' } })
    const msg = ch.addMessage({ encoding: cc.string, onmessage() { msg.send('R') } })
    ch.open()
    await new Promise((r) => setImmediate(r))
    written.length = 0
    const feed = (h) => stream.push(Buffer.from(h, 'hex')) // mux frames go as is: the long frame is made by secret-stream, not mux
    feed('000101017000') // first open the channel on the peer (remote id 1)
    await new Promise((r) => setTimeout(r, 30))
    const afterOpen = [...written]; written.length = 0
    feed(frameHex)
    await new Promise((r) => setTimeout(r, 60))
    results.push({ label, frameHex, writtenAfterOpen: afterOpen, writtenAfterFrame: written, streamDestroyed: stream.destroyed, closed, err })
    stream.destroy()
  }
  return { version: load(ver, 'protomux/package.json').version, note: 'writtenAfterFrame — bytes that mux wrote to the stream in response (without the frame length: secret-stream adds it)', frames: results }
}
const out = { d1, d12: { new: d12('new'), old: d12('old') }, d13: { new: await d13('new'), old: await d13('old') } }
const p = writeVectors('deltas', 'codec', { note: 'D1 (NS_PLUGIN, pluginRequest, NS prefixes), D12 (c.uint, c.buffer on compact-encoding 3.5.2 = new, 2.19.2 = old), D13 (protomux frames on 3.12.1 = new, 3.10.1 = old).', ...out }, ['hyperdht', 'compact-encoding', 'protomux', 'hypercore-crypto'])
console.log(p)
process.exit(0)
