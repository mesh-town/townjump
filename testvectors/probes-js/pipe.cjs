const NM = require('path').join(__dirname, '../../.tmp/npm-holesail/node_modules'); // JS reference holesail 2.4.4: interop/setup-js.sh
// Marks for pipe: udp_server_close_tests.rs:34/:93/:110, udp_proxy_tests.rs:32/:241, tcp_proxy_tests.rs:281, piper_tests.rs:227 (the files are in crates/townjump-pipe/src/).
const lib = require(NM + '/@holesail/hyper-cmd-lib-net')
const dgram = require('dgram')
const net = require('net')
const { Duplex } = require('stream')
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
const frame = (s) => { const b = Buffer.from(s); const h = Buffer.alloc(4); h.writeUInt32BE(b.length); return Buffer.concat([h, b]) }
function fakeRemote () { const d = new Duplex({ read () {}, write (c, e, cb) { d.written.push(c); cb() } }); d.written = []; d.events = []; for (const ev of ['end', 'close', 'error', 'finish']) d.on(ev, () => d.events.push(ev)); return d }
function logger () { const lines = []; return { lines, log: ({ type, msg }) => lines.push(['DEBUG', 'INFO', 'WARN', 'ERROR'][type] + ' ' + msg) } }
async function service () { const s = dgram.createSocket('udp4'); s.msgs = []; s.on('message', (m, r) => s.msgs.push([m.toString(), r])); await new Promise((r) => s.bind(0, '127.0.0.1', r)); return s }
;(async () => {
  const out = {}
  // 1. UDP framed server
  { const svc = await service(); const remote = fakeRemote(); const lg = logger()
    lib.pipeUdpFramedServer(remote, { port: svc.address().port, host: '127.0.0.1' }, lg, {})
    remote.push(frame('hello')); await sleep(100)
    svc.send(Buffer.from('12345'), svc.msgs[0][1].port, '127.0.0.1'); await sleep(100)
    remote.push(Buffer.concat([frame('a'), frame('bb')])); await sleep(100)
    out.udpServer_normal = { serviceGot: svc.msgs.map((m) => m[0]), writtenToRemoteBytes: remote.written.map((b) => b.length), log: [...lg.lines] }
    lg.lines.length = 0; const before = svc.msgs.length
    remote.emit('error', new Error('boom')); await sleep(150)
    remote.push(frame('after-error')); await sleep(100)
    out.udpServer_afterStreamError = { writableEnded: remote.writableEnded, events: remote.events, newDatagrams: svc.msgs.length - before, log: [...lg.lines] }
    svc.close() }
  { const svc = await service(); const remote = fakeRemote(); const lg = logger()
    lib.pipeUdpFramedServer(remote, { port: svc.address().port, host: '127.0.0.1' }, lg, {})
    remote.push(null); await sleep(150)
    out.udpServer_afterRemoteEnd = { writableEnded: remote.writableEnded, events: remote.events, log: [...lg.lines] }
    svc.close() }
  // 2. UDP proxy on a busy port
  { const holder = await service(); const lg = logger(); let bound = false
    let thrown = null
    try { lib.createUdpFramedProxy({ port: holder.address().port, host: '127.0.0.1' }, () => fakeRemote(), lg, () => { bound = true }) } catch (e) { thrown = { name: e.name, code: e.code, message: e.message } }
    await sleep(200)
    out.udpProxy_busyPort = { thrownSynchronously: thrown, onBindCalled: bound, log: [...lg.lines] }
    holder.close() }
  // 3. UDP proxy: closing the sender stream
  { const lg = logger(); const remotes = []; let ready
    const r = lib.createUdpFramedProxy({ port: 0, host: '127.0.0.1' }, () => { const d = fakeRemote(); remotes.push(d); return d }, lg, () => { ready = true })
    await sleep(100); const port = r.proxySocket.address().port
    const app = dgram.createSocket('udp4'); app.send(Buffer.from('hi'), port, '127.0.0.1'); await sleep(100)
    remotes[0].push(null); await sleep(150)
    const afterEnd = { events: [...remotes[0].events], clients: r.clients.size, log: lg.lines.filter((l) => /close|Remote/.test(l)) }
    remotes[0].destroy(); await sleep(100)
    out.udpProxy_remoteEndVsDestroy = { afterPushNull: afterEnd, afterDestroy: { events: [...remotes[0].events], clients: r.clients.size, log: lg.lines.filter((l) => /close|Remote/.test(l)) } }
    app.close(); r.proxySocket.close() }
  // 4. TCP proxy (client): directions in the log
  { const lg = logger(); const remotes = []; const stats = {}
    const proxy = lib.createTcpProxy({ port: 0, host: '127.0.0.1' }, () => { const d = fakeRemote(); remotes.push(d); return d }, { logger: lg }, stats, () => {})
    await sleep(100); const c = net.connect(proxy.address().port, '127.0.0.1'); c.on('error', () => {}); await sleep(100)
    remotes[0].push('answer'); await sleep(100); c.write('abcd'); await sleep(100)
    out.tcpProxy_directions = lg.lines.filter((l) => /Data from/.test(l)); c.destroy(); proxy.close() }
  // 5. connPiper: loc = local socket (server): directions
  { const lg = logger(); const remote = fakeRemote(); const loc = new (require('events'))(); loc.write = () => {}; loc.end = () => {}; loc.destroy = () => {}
    lib.connPiper(remote, () => loc, { logger: lg }, {})
    loc.emit('data', Buffer.from('12345')); remote.push('1234567'); await sleep(100)
    out.connPiper_directions = lg.lines.filter((l) => /Data from/.test(l)); out.connPiper_remoteWrittenFromLoc = remote.written.map(String) }
  console.log(JSON.stringify(out, null, 1)); process.exit(0)
})()
