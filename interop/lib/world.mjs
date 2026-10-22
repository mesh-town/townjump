// Environment of one scenario: the echo service, the server (real CLI), the clients; everything is stopped in cleanup().
// The server starts with --log 0: the line `Incoming connection received from` (DEBUG) is the only observable
// sign that the connection reached the server (it is absent on a firewall rejection).
import crypto from 'node:crypto'
import path from 'node:path'
import { startEcho } from '../js/echo.mjs'
import { BANNER_WAIT, parseBannerUrl } from './banner.mjs'
import { childEnv, commandFor } from './impl.mjs'
import { freeTcpPort, freeUdpPort } from './ports.mjs'
import { Proc, sleep } from './proc.mjs'

const READY_MS = 30_000
export const randomHex = (n) => crypto.randomBytes(n).toString('hex')

export class World {
  constructor(ctx, pair, scenario) {
    this.ctx = ctx
    this.pair = pair
    this.scenario = scenario
    this.procs = []
    this.tag = `${pair.client}-to-${pair.server}.${scenario.id}`
  }

  spawn(role, impl, argv, extraEnv) {
    const { cmd, args } = commandFor(impl, argv, this.ctx.rustBin)
    const logFile = path.join(this.ctx.outDir, `${this.tag}.${role}.log`)
    const proc = new Proc(`${impl} ${role}`, cmd, args, { env: childEnv(this.ctx.bootstrap, extraEnv), logFile })
    this.procs.push(proc)
    return proc
  }

  async startServer() {
    const { proto, mode, phrase } = this.scenario
    this.echo = await startEcho()
    const target = proto === 'udp' ? this.echo.udpPort : this.echo.tcpPort
    const argv = ['--live', String(target), '--log', '0']
    if (proto === 'udp') argv.push('--udp')
    if (mode === 'public') argv.push('--public')
    this.phrase = mode === 'private' && phrase ? `interop-${randomHex(16)}` : null
    if (this.phrase) argv.push('--key', this.phrase)
    this.server = this.spawn('server', this.pair.server, argv, this.ctx.serverEnv)
    await this.server.waitFor(BANNER_WAIT, READY_MS)
    await this.server.waitFor(/Server listening on key/, READY_MS)
    this.readBanner()
  }

  readBanner() {
    const banner = parseBannerUrl(this.server.text)
    const { mode } = this.scenario
    if (banner.secure !== (mode === 'private')) throw new Error(`server banner: ${banner.url} does not match mode ${mode}`)
    if (this.phrase && banner.key !== this.phrase) throw new Error(`server banner: key ${banner.key}, but ${this.phrase} was set`)
    this.url = banner.url
    this.privateKey = mode === 'private' ? banner.key : null
  }

  // Client by link; the port and protocol come from the DHT record (--udp is not passed to the client), the client port is free.
  // Returns settleMs after the "Proxy listening" line.
  async startClient(url) {
    const port = this.scenario.proto === 'udp' ? await freeUdpPort() : await freeTcpPort()
    const extraEnv = this.scenario.clientEnv
    const proc = this.spawn('client', this.pair.client, [url, '--port', String(port), '--log', '1'], extraEnv)
    await proc.waitFor(new RegExp(`Proxy listening on 127\\.0\\.0\\.1:${port}`), READY_MS)
    await sleep(this.ctx.settleMs) // a hyperdht node right after start does not know its address yet: without the pause ~1 in 10 first connections on localhost fails
    return { proc, port }
  }

  expectIncoming(n) {
    const got = this.server.count(/Incoming connection received from/)
    if (got !== n) throw new Error(`server: ${got} lines «Incoming connection received from», expected ${n}\n${this.server.tail(8)}`)
  }

  diagnostics() {
    return this.procs.map((p) => p.tail(6)).join('\n')
  }

  async cleanup() {
    await Promise.all(this.procs.map((p) => p.stop()))
    await this.echo?.close()
  }
}
