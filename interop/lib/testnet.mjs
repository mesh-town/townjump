// Launch of js/testnet.mjs: a local testnet on 127.0.0.1, the bootstrap port is from the first output line.
import path from 'node:path'
import { Proc } from './proc.mjs'
import { ROOT_DIR } from './impl.mjs'

export async function startTestnet(nodes, logFile) {
  const script = path.join(ROOT_DIR, 'interop/js/testnet.mjs')
  const proc = new Proc('testnet', process.execPath, [script, String(nodes), '--exit-on-stdin-eof'], { logFile })
  const [, bootstrap] = await proc.waitFor(/TOWNJUMP_BOOTSTRAP=(\S+)/, 60_000)
  return { proc, bootstrap }
}
