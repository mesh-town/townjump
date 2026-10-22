# Client: `holesail-client` 2.3.0

Sources: `holesail-client/index.js:HolesailClient` (constructor, `connect`, `handleTCP`, `handleUDP`, `get`, `info`, `pause`, `resume`, `destroy`, `ping`), `holesail/src/index.js:Holesail` (`_open`, `connect`, `info`). The `--connect` CLI branch / positional key — `holesail/src/bin/holesail.mjs`.

## Input

CLI: `key = argv.connect || argv._[0]`; `new Holesail({ client: true, port: argv.port, host: argv.host, key, udp: argv.udp, secure: argv.public, log })`. The mode and key pair — [connection-string.md](connection-string.md) (on the client `--public` is inverted, quirk 1). `Holesail._open` creates `new HolesailClient({ key: seed, secure, logger })` and calls `connect({ port, host, udp })`; `port` and `host` are the flag values or `undefined`.

## The `HolesailClient` constructor

- `secure` — private: `keyPair = HyperDHT.keyPair(z32.decode(seed))` (here `seed` is the z32 string `sha256(K)`), `publicKey = keyPair.publicKey`. Public: `publicKey = z32.decode(key)`, `keyPair` is unset. A key outside the z32 alphabet throws `Invalid character in base32 input: "!" at position 3` (quirk 1, quirk 12); it comes from `ready()`, not from `new Holesail` (the `HolesailClient` constructor is created in `_open`), before the DHT node is created (`new` does not throw, `ready()` rejects, nothing is logged).
- `this.dht = new HyperDHT({ keyPair })`: in private mode the node identity is the derived pair (the server tells the client apart by it, [firewall.md](firewall.md)); in public mode it is a random one-time node pair.

## `connect(options)` — order

1. INFO `Connecting to key: <seed>, secure: <bool>` (quirk 9: in private mode `seed` is `z32(sha256(K))`, equal to the secret).
2. `get()` → `dht.mutableGet(publicKey, {})` (in hyperdht `latest` is on by default, `{}` is equivalent to `{ latest: true }`, see [dht-records.md](dht-records.md)); DEBUG `Getting DHT record`; if found — DEBUG `DHT get completed: seq=<n>, value=<text>`, otherwise WARN `DHT get: no record found`. The record is read by `publicKey`: in private mode by the DERIVED public key (the one under which the server put the record), in public mode by the key from the string.
3. Record found → `dhtData = JSON.parse(value)` (invalid JSON or `null` → an exception, code 1, quirk 12), DEBUG `Retrieved DHT data: <JSON.stringify(dhtData)>` — the record after parsing and re-serialization, not the original text (for example `{"host":"127.0.0.1", "port": 1e2}` → `{"host":"127.0.0.1","port":100}`; integer keys come first in ascending order; `1.50` → `1.5`, `-0` → `0`, `1E+30` → `1e+30`, a duplicate key — the last value, `12345678901234567890` → `12345678901234567000`); not found → `dhtData = {}`, WARN `No DHT data retrieved`.
4. Merge (the `??` operator — only `null`/`undefined` yield, `0` and `''` are kept):
   - `port = options.port ?? dhtData.port ?? 8989`
   - `host = options.host ?? dhtData.host ?? '127.0.0.1'`
   - `udp  = options.udp  ?? dhtData.udp  ?? false`
   This is quirk 11: without flags the client listens ON THE SAME `host:port` that the server forwards to on its side (for example `127.0.0.1:3000`), and takes the protocol from the record. Port 8989 from the help text is only a fallback.
5. `this.args = options`, `state = 'waiting'`; `options.udp` falsy → `handleTCP`, otherwise `handleUDP`. `connect` returns right after the handler is started, not after `listen` (quirk 8).

## TCP (`handleTCP`)

DEBUG `Handling TCP connection`; `proxy = createTcpProxy({ port, host }, () => dht.connect(publicKey, { reusableSocket: true }), { compress: false, logger }, stats, onListen)`. The local TCP server listens on `host:port`; for every accepted local connection a NEW DHT connection (`dht.connect`) is created, then `connPiper` ([pipe.md](pipe.md)). After a successful `listen`: `state = 'listening'`, INFO `Proxy listening on <host>:<port>`. A `listen` error (busy port, unavailable address) is a server `error` event without a handler → an unhandled exception, code 1 (`conn-port-inuse`, `conn-record-host-port`), the banner has already been printed by then (quirk 8).

## UDP (`handleUDP`)

DEBUG `Handling UDP connection`; `createUdpFramedProxy({ port, host }, () => dht.connect(publicKey), logger, onBind)` (without `reusableSocket`); `proxy` is a local `udp4` socket, `clients` is a map of streams by source. After `bind`: INFO `Proxy listening on <host>:<port> for UDP`. Frames and the stream lifecycle — [pipe.md](pipe.md).

## The rest of the API

| Method | Action |
|---|---|
| `get(opts)` | as step 2; returns `{ seq, value }` (the value is text) or `null` |
| `info` | `{ type:'client', state, secure, port, host, protocol:'udp'\|'tcp', key: seed, publicKey: z32(publicKey) }` (before `connect` `info` throws `TypeError: Cannot read properties of undefined (reading 'port')`: `this.args` does not exist yet; from the CLI this is unreachable — `info` is read after `ready`; in Rust `info()` does not return `Result`, before `connect` the state is `None`); `Holesail.info` replaces `key` with the original `K` for private mode and adds `seed = hex(sha256(K))` |
| `pause()`, `resume()` | `dht.suspend()`/`dht.resume()`, `state` `'paused'`/`'listening'`, INFO `Pausing client`/`Client paused`/`Resuming client`/`Client resumed` |
| `destroy()` | INFO `Destroying client`; `await dht.destroy()`; closes the local proxy; for UDP — `remoteStream.destroy()` for each client and clearing the map; `state='destroyed'`; INFO `Client destroyed` |
| `HolesailClient.ping(key, dht?)` | looking up a record by key, [dht-records.md](dht-records.md) |

The client has no reconnection logic: when a DHT connection drops, only the local connection it served is closed ([pipe.md](pipe.md)). If there is no server, `dht.connect` fails, the local connection is closed, and the client keeps listening.

## Client log messages (for `--log`)

INFO: `Connecting to key: <seed>, secure: <bool>`, `Proxy listening on <host>:<port>` (with `for UDP` for UDP). DEBUG: `Getting DHT record`, `DHT get completed: seq=<n>, value=<text>`, `Retrieved DHT data: <JSON.stringify(dhtData)>`, `Handling TCP connection`/`Handling UDP connection`. WARN: `DHT get: no record found`, `No DHT data retrieved`. Plus the piping messages ([pipe.md](pipe.md)). Order: `connect` messages → banner → `Proxy listening on …` (after the banner, when `listen` completes).

## Fork error map (`map_error` in `townjump-dht-peeroxide`)

A JS DHT error message is `<CODE>: <text>` (`hyperdht/lib/errors.js:DHTError`); Node socket errors are a plain `Error` with a `code` (`ECONNRESET` etc.), not a `DHTError`.
The user sees the text in stderr, so the Rust version keeps the text identical. The `HyperDhtError` branches of the peeroxide fork:

| Fork branch | `DhtError` | Text (`Display`) |
|---|---|---|
| `Destroyed`, `Dht(Destroyed)` | `Destroyed` | `NODE_DESTROYED: Node destroyed` |
| `PeerNotFound` | `PeerNotFound` | `PEER_NOT_FOUND: Peer not found` |
| `HolepunchAborted` | `HolepunchAborted` | `HOLEPUNCH_ABORTED: Holepunch aborted` |
| `HandshakeFailed(text)` | `Backend` | `BAD_HANDSHAKE_REPLY: <text>` |
| `StreamEstablishment(text)` | `ConnectionFailed(text)` | `PEER_CONNECTION_FAILED: <text>` |
| `HolepunchFailed` | `Backend` | `CANNOT_HOLEPUNCH: Cannot holepunch to remote` |
| `NoRelayNodes` | `ConnectionFailed("Could not connect to peer")` | `PEER_CONNECTION_FAILED: Could not connect to peer` |
| socket error (`Udx`, `Io`) | `Io` | as in Node: `<code> <text>` |

Default codes and texts come from `hyperdht/lib/errors.js` (`BAD_HANDSHAKE_REPLY`, `CANNOT_HOLEPUNCH`, `PEER_CONNECTION_FAILED`). The four middle branches
have no reference run, since the peeroxide fork has no JS counterpart; their codes were chosen by meaning.
