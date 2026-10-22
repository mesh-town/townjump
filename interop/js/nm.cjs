// Where the reference npm tree of holesail 2.4.4 lies (installed by interop/setup-js.sh). Overridden by TOWNJUMP_JS_NM.
// Common harness helper: js/bootstrap-env.cjs, js/testnet.mjs, lib/js-keys.mjs. We do not edit files in node_modules.
const path = require('path')
const fs = require('fs')

const ROOT = path.resolve(__dirname, '../..')
const NM = process.env.TOWNJUMP_JS_NM || path.join(ROOT, '.tmp/npm-holesail/node_modules')
const CLI = path.join(NM, 'holesail/src/bin/holesail.mjs')

if (!fs.existsSync(CLI)) {
  throw new Error(`no JS holesail reference (${CLI}): run interop/setup-js.sh or set TOWNJUMP_JS_NM`)
}

exports.ROOT = ROOT
exports.NM = NM
exports.CLI = CLI
exports.req = (name) => require(path.join(NM, name))
