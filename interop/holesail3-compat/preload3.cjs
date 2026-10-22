// Preload for the holesail 3.0 CLI: substitutes the local testnet bootstrap into HyperDHT.BOOTSTRAP
// of exactly the hyperdht that lies in .tmp/holesail-3.0 (interop/js/bootstrap-env.cjs patches the hyperdht from the 2.4.4 tree).
const path = require('path')
const HyperDHT = require(path.resolve(__dirname, '../../.tmp/holesail-3.0/node_modules/hyperdht'))
const raw = process.env.TOWNJUMP_BOOTSTRAP
if (raw) {
  const nodes = raw.split(',').map((s) => s.trim()).filter(Boolean)
  HyperDHT.BOOTSTRAP.splice(0, HyperDHT.BOOTSTRAP.length, ...nodes)
}
