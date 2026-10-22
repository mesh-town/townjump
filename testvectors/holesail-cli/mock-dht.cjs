// Preload (node -r): replaces hyperdht with a stub without a network and writes a trace of calls to $MOCK_TRACE (JSON lines).
// Randomness substitution: $MOCK_RANDOM_HEX (64 hex) instead of hyper-cmd-lib-keys.randomBytes(32).
const Module = require('module')
const fs = require('fs')
const EventEmitter = require('events')
const origLoad = Module._load
const TRACE = process.env.MOCK_TRACE
const hex = (b) => (b ? Buffer.from(b).toString('hex') : b)
function trace (ev, data) { if (TRACE) fs.appendFileSync(TRACE, JSON.stringify({ ev, ...data }) + '\n') }
let real = null
function realDht (parent) { return real || (real = origLoad('hyperdht', parent, false)) }

function makeFake (parent) {
  const Real = realDht(parent)
  class FakeDHT extends EventEmitter {
    constructor (opts = {}) { super(); trace('HyperDHT.new', { keyPair: opts.keyPair ? { publicKey: hex(opts.keyPair.publicKey) } : null, opts: Object.keys(opts) }) }
    static keyPair (seed) { const kp = Real.keyPair(seed); trace('HyperDHT.keyPair', { seed: hex(seed), publicKey: hex(kp.publicKey) }); return kp }
    createServer (opts, onconn) {
      const fw = opts && opts.firewall
      trace('createServer', { firewall: typeof fw, reusableSocket: opts && opts.reusableSocket, optKeys: Object.keys(opts || {}) })
      return { on () {}, listen: async (kp) => {
        trace('server.listen', { publicKey: hex(kp.publicKey) })
        if (typeof fw === 'function') {
          const own = fw(kp.publicKey)
          const other = fw(Buffer.alloc(32, 7))
          trace('firewall.probe', { ownKey: own, otherKey: other })
        }
        if (process.env.MOCK_FAKE_CONN) onconn(Object.assign(new EventEmitter(), { remotePublicKey: Buffer.alloc(32, 9), write () {}, end () {}, destroy () {} }))
      } }
    }
    async mutableGet (pk, opts) {
      trace('mutableGet', { publicKey: hex(pk), opts })
      const rec = process.env.MOCK_RECORD_FOR === hex(pk) || process.env.MOCK_RECORD_FOR === '*' ? process.env.MOCK_RECORD : null
      return rec ? { seq: 3, value: Buffer.from(rec), signature: Buffer.alloc(64) } : null
    }
    async mutablePut (kp, value, opts) {
      trace('mutablePut', { publicKey: hex(kp.publicKey), value: Buffer.from(value).toString(), seq: opts && opts.seq, optKeys: Object.keys(opts || {}) })
      return { seq: (opts && opts.seq) || 0 }
    }
    connect (pk, opts) {
      trace('connect', { publicKey: hex(pk), opts })
      return Object.assign(new EventEmitter(), { write () { return true }, end () {}, destroy () {} })
    }
    async destroy () { trace('dht.destroy', {}) }
    async suspend () { trace('dht.suspend', {}) }
    async resume () { trace('dht.resume', {}) }
  }
  return FakeDHT
}
let fake = null
Module._load = function (request, parent, isMain) {
  if (request === 'hyperdht') return fake || (fake = makeFake(parent))
  const m = origLoad.apply(this, arguments)
  if (request === 'hyper-cmd-lib-keys' && process.env.MOCK_RANDOM_HEX && !m.__mocked) {
    m.randomBytes = (n) => { trace('randomBytes', { n }); return Buffer.from(process.env.MOCK_RANDOM_HEX, 'hex').subarray(0, n) }
    m.__mocked = true
  }
  return m
}
