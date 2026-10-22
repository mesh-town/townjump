# Stream piping: `@holesail/hyper-cmd-lib-net` 1.1.2

Sources: `@holesail/hyper-cmd-lib-net/index.js:connPiper`, `createTcpProxy`, `pipeTcpServer`, `createUdpFramedProxy`, `pipeUdpFramedServer`. The module also exports `UdpSocket`, `UdpConnPiper`, `udpPiper`, `udpConnect`; holesail does NOT use them — there is no need to implement them. The layer below (the encrypted duplex stream of a DHT connection, `dht.connect` and the server's incoming `c`) is the peeroxide fork; here it is a "byte stream with the events `data`, `end`, `error`, `close` and the methods `write`, `end`, `destroy`". UDP sockets in JS are `bare-dgram` (compatible with Node `dgram`), family `udp4`.

## TCP: `connPiper(connection, dst, opts, stats)`

Pipes TWO streams together, `connection` (on the client side a local TCP socket, on the server side a DHT stream) and `loc = dst()` (client: a new DHT connection, server: a new TCP connection to the target service).

- `dst()` returned `null` → WARN log `Connection rejected (null destination)`, `connection.destroy()`, `stats.rejectCnt += 1`, exit. Holesail never returns `null`.
- Otherwise `stats.locCnt += 1`, `stats.remCnt += 1` (the counters are created as zero by default and decremented on destruction; they are not shown anywhere in the CLI).
- Data: `loc.on('data', d) → connection.write(d)` and `connection.on('data', d) → loc.write(d)`; the return value of `write` is not checked, there are no pauses and no buffer limit.
- Half-close: `loc` `end` → `connection.end()`; `connection` `end` → `loc.end()` (a half-close is propagated; TCP sockets have `allowHalfOpen: true`, so the opposite end keeps working until it is closed).
- Destruction (once): any `error` or `close` of either stream calls `destroy(err)`: counters −1, `loc.end()`, `connection.end()`, `loc.destroy(err)`, `connection.destroy(err)`, then `opts.onDestroy?.(err)`. That is, closing one side destroys both at once; data in flight may be lost.
- An error connecting to the target service (`ECONNREFUSED` on the server) — `error` on `loc` → destruction → the DHT stream is closed; the client will see the close.
- Log: INFO `Starting TCP connection piper`; DEBUG `Data from local to remote: <n> bytes` (data from `loc`) and `Data from remote to local: <n> bytes` (data from `connection`; `n` is the length of the `data` chunk) — the "local/remote" labels are fixed relative to `loc`/`connection` and on the client side read as reversed; DEBUG `Local end, ending connection`, `Connection end, ending local`; INFO `Connected` (the `connect` event on `loc`; a DHT stream may lack the event); INFO `Destroying connection piper` (+ ` due to error: <msg>`).
- Returns `{}`.

### Client: `createTcpProxy(listenOpts, connectRemote, piperOpts, stats, onListen)`

`net.createServer({ allowHalfOpen: true }, c => connPiper(c, connectRemote, piperOpts, stats))`; `proxy.listen(port, host, onListen)`. A `listen` error is an unhandled `error` (code 1). There is no limit on the number of connections. For every accepted socket `connectRemote()` is called at once, before the DHT handshake completes; data written before readiness is buffered by the stream.

### Server: `pipeTcpServer(remoteStream, localOpts, piperOpts, stats)`

`connPiper(remoteStream, () => net.connect({ port: +localOpts.port, host: localOpts.host, allowHalfOpen: true }), …)`. The port is converted to a number by unary plus. The host is as is (a name or an address). The options `isServer`, `compress` are ignored. Data from the client before the local connection is established is buffered by the socket.

Not set: timeouts, keepalive, `noDelay` (Node default — Nagle is on), buffer limits.

## UDP: the frame

Datagrams travel over the reliable ordered DHT stream as frames: `[length: 4 bytes, big-endian, unsigned][datagram body]`. The length covers the body only. The frame does not limit the length, but a datagram that holesail receives on the local UDP socket (client: from the application, server: the service's reply) is truncated to 2048 bytes and the remainder is lost: that is how `bare-dgram` over `udx-native` works (a 4000-byte datagram arrives as 2048 bytes). The Rust version reproduces this, following the principle of matching holesail exactly. The Rust receive loops take a datagram of any size whole into a 64 KiB buffer and cut it to 2048 bytes after reception, as libuv does (`uv__process_udp_recv_req` hands over an oversized datagram truncated); with a 2048-byte buffer `recv_from` on Windows fails with WSAEMSGSIZE (10040) and loses both the datagram and its sender address. An empty datagram is a frame of length 0 (received and sent as an empty datagram). The receiver accumulates bytes in a buffer and loops: while `buffer ≥ 4` it reads the length `L`; if `buffer < 4 + L` it waits, otherwise it hands out the body `[4, 4+L)` and discards `4+L` bytes. Frames are not reordered: datagrams arrive reliably and in order thanks to the stream (UDP loss and reordering disappear, head-of-line blocking is possible).

### Client: `createUdpFramedProxy(listenOpts, connectRemote, logger, onBind)`

- One local `udp4` socket: `bind(port, host, onBind)`. An asynchronous socket error — ERROR log `Proxy socket error: <stack>` and `close()` of the socket; the process does NOT crash (unlike TCP). On Windows the ICMP echo of our own send to a closed local UDP port (WSAECONNRESET 10054, WSAECONNABORTED 10053, WSAENETRESET 10052) is not such an error: libuv (`src/win/udp.c`, `uv__process_udp_recv_req`) skips 10054 and 10052, and libudx drops negative reads, so the JS loop never ends on them; on 10053 libuv stops receiving (`uv_udp_recv_stop`) and the JS socket goes silently deaf. The Rust receive loop skips all three and keeps reading (a deliberate deviation for 10053: a deaf socket is worse), logging one DEBUG line per skipped echo and pausing 10 ms between reads after 256 echoes in a row. `bind` on a busy port does not get here: it throws SYNCHRONOUSLY from `createUdpFramedProxy` (`Error`, `code: 'EADDRINUSE'`, `message: 'address already in use'`), with no log and no `onBind` (confirmed with `js/pipe.cjs`).
- The `clients` map: the key is `"<address>:<port>"` of the datagram's sender. The first datagram from a new sender creates `remoteStream = connectRemote()` (i.e. a separate DHT connection per local sender, without `reusableSocket`), an entry `{ remoteStream, rinfo, buffer }`; replies from the stream are parsed into frames and sent with `proxySocket.send(body, rinfo.port, rinfo.address)` to THIS sender (the address is remembered from the first datagram). A send error — ERROR log `Send error to <id>: <stack>`.
- Every datagram from the sender is written to its `remoteStream` as a frame.
- `remoteStream` `error`: ERROR log `Remote error for <id>: <stack>`, the entry is removed, the stream is destroyed; `close`: DEBUG `Remote close for <id>`, the entry is removed. There is no idle timeout and no limit on the number of senders.
- DEBUG log: `UDP message from <id>: <n> bytes`, `UDP response for <id>: <n> bytes`.
- Returns `{ proxySocket, clients }`; the client's `destroy` closes the socket and destroys the streams.

### Server: `pipeUdpFramedServer(remoteStream, localOpts, logger, stats)`

- For EACH incoming DHT connection — one local socket of its own, implicitly bound (`udp4`, the port is chosen by the system on the first send).
- From the stream: frames are parsed in a loop and sent with `localSocket.send(body, +localOpts.port, localOpts.host)` (an error — ERROR `Send error to local: <stack>`).
- From the socket: any received datagram (from any address, no filter) is packed into a frame and written to the stream; DEBUG `Data from local to remote: <n> bytes`, `Data from remote to local: <n> bytes` (for a stream chunk); the sizes are in "Clarifications".
- Closing: stream `end` → close the socket; socket `close` → `remoteStream.end()`; stream `error` → close the socket; a socket error — ERROR `Local UDP socket error: <stack>` and `remoteStream.destroy(err)` (the handler is attached twice). The same ICMP echo (10054/10053/10052) after a send to a closed service port is not a socket error: the receive loop skips it and keeps reading (see the client for 10053 and the DEBUG line). The receive buffer is 64 KiB per incoming DHT connection. Closing the stream (`destroy`) without `end`/`error` does NOT close the socket — the stream has no `close` handler (per the source; not verified at runtime, not reachable from holesail except by a DHT drop).
- `stats` is not used for UDP.

## Clarifications (from `testvectors/probes-js/pipe.cjs`)

- **The size in `Data from … bytes`.** `Data from remote to local: <n>` (on the server) is the length of the WHOLE stream CHUNK, with the 4-byte headers: the frame `hello` (4 + 5) → `9`; two frames
  (`a`, `bb`) in one chunk → `11`; the line is written for every chunk before frame parsing. `Data from local to remote: <n>` is the length of the datagram WITHOUT the header (`12345` → `5`), while 4 + n = 9
  goes into the stream. In `connPiper` `n` is the chunk length (`5`, `7`); on the client TCP proxy the directions are those of `loc` = the DHT stream: a reply from DHT (6 bytes) → `local to remote: 6`,
  bytes from the local socket (4) → `remote to local: 4`.
- **UDP server, stream error:** `error` → the socket is closed → the socket's `close` fires → `remoteStream.end()`; after that no datagrams go to the service (0 new). The stream's `end` → the socket
  is closed → also `remoteStream.end()`, the stream events `end`, `finish`, `close`.
- **UDP client proxy, the sender stream's end:** `end` (FIN) does not remove the `clients` entry and writes no line; removal and DEBUG `Remote close for <id>` come only from the stream's `close`
  (`destroy`). Rust matches JS: the client is removed when the stream closes entirely, not on read EOF (Rust's reader EOF distinguishes a clean end from a break and an error).

## What must not change in a port

The frame format, the behavior "one DHT connection per TCP connection / per UDP sender", "one UDP socket per incoming connection" on the server, the TCP half-close, destruction of both sides on `error`/`close` of either. Not part of the contract (can be improved without losing wire compatibility): the absence of backpressure.
