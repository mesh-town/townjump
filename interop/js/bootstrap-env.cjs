// Preload for the JS CLI: `node -r interop/js/bootstrap-env.cjs <holesail.mjs> ...`.
// holesail has no bootstrap option (`new HyperDHT()` without parameters), and the HyperDHT.BOOTSTRAP array
// is passed to the constructor by reference, so we patch it in place. The format is as for the Rust binary:
// TOWNJUMP_BOOTSTRAP=host:port[,host:port...]. Without the variable the JS behavior is unchanged.
// INTEROP_JS_FIREWALL=unknown — emulation of a client behind NAT (remoteAddress() = null): without it on localhost
// the client is always OPEN and the holepunch path is not taken (to test the holepunch path we emulate NAT with this variable).
const { req } = require('./nm.cjs')

const HyperDHT = req('hyperdht')

const raw = process.env.TOWNJUMP_BOOTSTRAP
if (raw) {
  const nodes = raw.split(',').map((s) => s.trim()).filter(Boolean)
  HyperDHT.BOOTSTRAP.splice(0, HyperDHT.BOOTSTRAP.length, ...nodes)
}

if (process.env.INTEROP_JS_FIREWALL === 'unknown') {
  HyperDHT.prototype.remoteAddress = () => null
}

// INTEROP_JS_MUTATE=open-firewall — harness self-check: the JS server admits any client, and the scenario
// fw3-public-client must fail (run.mjs --mutate open-firewall). Only for checking the harness itself.
if (process.env.INTEROP_JS_MUTATE === 'open-firewall') {
  const createServer = HyperDHT.prototype.createServer
  HyperDHT.prototype.createServer = function (opts = {}, onconnection) {
    if (typeof opts === 'function') return createServer.call(this, { firewall: () => false }, opts)
    return createServer.call(this, { ...opts, firewall: () => false }, onconnection)
  }
}
