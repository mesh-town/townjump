// Child processes of the harness. The output (stdout and stderr together, without ANSI) accumulates in memory and is written to a file;
// waitFor waits for a line in the output. All running processes are stopped when the runner exits, including on Ctrl-C.
import { spawn } from 'node:child_process'
import fs from 'node:fs'

const ANSI = /\x1b\[[0-9;]*[A-Za-z]/g
export const stripAnsi = (s) => s.replace(ANSI, '')
export const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

const live = new Set()
const killAll = () => { for (const p of live) p.child.kill('SIGKILL') }
process.on('exit', killAll)
for (const sig of ['SIGINT', 'SIGTERM']) process.on(sig, () => { killAll(); process.exit(130) })

export class Proc {
  constructor(name, cmd, args, { env = process.env, logFile = null } = {}) {
    this.name = name
    this.text = ''
    this.exitCode = null
    this.waiters = []
    this.logStream = logFile ? fs.createWriteStream(logFile) : null
    this.logStream?.write(`$ ${cmd} ${args.join(' ')}\n`)
    this.exited = new Promise((resolve) => { this.markExited = resolve })
    // stdin is a pipe we do not close: the process sees no EOF until it is stopped
    this.child = spawn(cmd, args, { env, stdio: ['pipe', 'pipe', 'pipe'] })
    live.add(this)
    for (const stream of [this.child.stdout, this.child.stderr]) stream.on('data', (d) => this.onData(d))
    this.child.on('error', (e) => this.onExit(`spawn: ${e.message}`))
    this.child.on('exit', (code, signal) => this.onExit(signal ?? code))
  }

  onData(chunk) {
    const text = stripAnsi(chunk.toString('utf8'))
    this.text += text
    this.logStream?.write(text)
    this.checkWaiters()
  }

  onExit(code) {
    if (this.exitCode !== null) return
    this.exitCode = code
    live.delete(this)
    this.logStream?.end(`\n[exit ${code}]\n`)
    for (const w of this.waiters.splice(0)) {
      clearTimeout(w.timer)
      w.reject(new Error(`${this.name}: process exited (${code}) before ${w.re} appeared\n${this.tail(12)}`))
    }
    this.markExited()
  }

  checkWaiters() {
    for (const w of [...this.waiters]) {
      const match = this.text.match(w.re)
      if (!match) continue
      clearTimeout(w.timer)
      this.waiters.splice(this.waiters.indexOf(w), 1)
      w.resolve(match)
    }
  }

  waitFor(re, ms) {
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.waiters = this.waiters.filter((x) => x !== w)
        reject(new Error(`${this.name}: ${re} did not appear in ${ms} ms\n${this.tail(12)}`))
      }, ms)
      const w = { re, resolve, reject, timer }
      this.waiters.push(w)
      this.checkWaiters()
    })
  }

  count(re) {
    return (this.text.match(new RegExp(re.source, 'g')) ?? []).length
  }

  tail(n) {
    const lines = this.text.split('\n').filter((l) => l.trim() !== '')
    return lines.slice(-n).map((l) => `    [${this.name}] ${l.slice(0, 200)}`).join('\n')
  }

  async stop() {
    if (this.exitCode !== null) return
    this.child.kill('SIGTERM')
    await Promise.race([this.exited, sleep(3000)])
    if (this.exitCode === null) this.child.kill('SIGKILL')
    await this.exited
  }
}
