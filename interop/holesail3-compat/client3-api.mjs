// holesail 3.0 client through the API, without probe (the 3.0 CLI always sends MODE_PROBE for TCP, because host/udp are not set).
// Usage: node -r ./preload3.cjs client3-api.mjs <invite> <port>
import { createRequire } from 'node:module'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
const here = path.dirname(fileURLToPath(import.meta.url))
const require = createRequire(path.join(here, '../../.tmp/holesail-3.0/package.json'))
const HolesailClient = require('holesail-client')
const [invite, port] = process.argv.slice(2)
const noop = () => {}
const logger = { debug: (m) => console.log('[debug]', m), info: (m) => console.log('[info]', m), warn: (m) => console.log('[warn]', m), error: (m) => console.log('[error]', m) }
const c = new HolesailClient({ invite, port: +port, host: '127.0.0.1', udp: false, logger })
await c.ready()
console.log('READY api-client, no probe')
