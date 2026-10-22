// JS reset probe: like udx-echo-server.js, but without echo. Reads the stream silently and
// prints JSON events line by line: {"error": <code>}, {"end": true}.
// Needed by libudx/tests/async_stream_interop.rs: DESTROY from a Rust peer
// must give the JS side an ECONNRESET error, not a stream end.
const UDX = require('udx-native')
const readline = require('readline')

const u = new UDX()
const socket = u.createSocket()
socket.bind(0, '127.0.0.1')

const { port } = socket.address()
const LOCAL_ID = 1
const REMOTE_ID = 2

function print (obj) {
  process.stdout.write(JSON.stringify(obj) + '\n')
}

print({ port, localId: LOCAL_ID, remoteId: REMOTE_ID })

const rl = readline.createInterface({ input: process.stdin })
rl.once('line', (line) => {
  const info = JSON.parse(line)
  const remotePort = info.port

  const stream = u.createStream(LOCAL_ID)
  stream.connect(socket, REMOTE_ID, remotePort, '127.0.0.1')

  print({ ready: true })

  stream.resume()

  stream.on('error', (e) => {
    print({ error: e.code })
  })

  stream.on('end', () => {
    print({ end: true })
  })

  stream.on('close', () => {
    socket.close()
    process.exit(0)
  })
})
