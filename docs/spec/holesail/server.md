# Server: `holesail-server` 2.4.0 and the `Holesail` layer

Sources: `holesail-server/index.js:HolesailServer` (the constructor, `start`, `handleTCP`, `handleUDP`, `key`, `put`, `get`, `info`, `pause`, `resume`, `destroy`), `holesail/src/index.js:Holesail` (`_open`, `connect`, `info`). The CLI `--live` branch is `holesail/src/bin/holesail.mjs` (top level).

## Input

The CLI (`--live P`) builds `new Holesail({ server: true, port: argv.live, host: argv.host || '127.0.0.1', udp: argv.udp, secure, key: argv.key, log })`. The computation of `secure`, the seed and the key is in [connection-string.md](connection-string.md). `Holesail.connect()` calls `HolesailServer.start({ port, host, seed, secure, udp })`; a repeated call while `running` — `Error('Already connected')`. `Holesail.ready()` completes when `start` has returned, then the CLI prints the banner (quirk 8; [cli.md](cli.md)).

`port` is a number from minimist (`3000`, `3000.5`, `16` from `0x10`); `host` is a string, the target address (where the traffic is forwarded); `udp` is the flag value as is (`undefined`, `true`, `'false'`, a number).

Internally, it creates its own `new HyperDHT()` with default settings (the Holepunch network bootstrap nodes, a random one-time node identity). The SERVER's identity for clients is not that one but the `keyPair` with which `server.listen` is called.

## `start(args)` — order of actions

1. INFO log `Starting server`. `secure = (args.secure === true)` (strict comparison; `Holesail` always passes a boolean).
2. `generateKeyPair(args.seed)`: `seed` is a hex string (`sha256(K)` in hex, or `undefined` → 32 random bytes in hex, [connection-string.md](connection-string.md)); `this.seed = Buffer.from(seed, 'hex')`; `keyPair = HyperDHT.keyPair(this.seed)`; DEBUG log `Generated key pair from seed: <hex>` (quirk 9).
3. Private mode: the firewall predicate (below) and INFO `Using Private Mode`; public: `firewall: false`, INFO `Using Public Mode`.
4. `this.server = dht.createServer({ firewall, reusableSocket: true }, onConnection)`; DEBUG `Server created, awaiting listen`.
5. `server.listen(keyPair)` — WITHOUT `await`: `.then` sets `state = 'listening'` and writes INFO `Server listening on key: <key>` (`key` is a property, see below; in private mode it is `z32(sha256(K))`, quirk 9). A `listen` error is not caught (a rejected top-level promise → code 1, not checked against the code). `Holesail` has no callback.
6. `data = JSON.stringify({ host: args.host, udp: args.udp, port: args.port })`; DEBUG `Initializing DHT with host info: <data>`; `await this.put(data)` ([dht-records.md](dht-records.md)).
7. `setInterval` every 50 minutes (`50 * 60 * 1000` ms): DEBUG `Refreshing DHT record: <data>` and a repeated `put(data)`.

`start` returns after step 6, not after `listen` (quirk 8): on a real network the banner can be seen before the node has started accepting connections.

## The firewall predicate (private mode only)

```
firewall(remotePublicKey) := !equals(remotePublicKey, keyPair.publicKey)     // true = refuse
```

The only input parameter that holesail uses is the client's public key from the handshake; the comparison is byte equality with the public key of the server's key pair. A private-mode client builds the same pair from `sha256(K)`, so it passes; any other key, including a public client, is refused. In public mode `firewall: false` (the lower layer substitutes "block no one"). The moment and data of the call, and the behavior on refusal — [firewall.md](firewall.md).

## Incoming connection (`onConnection(c)`)

`c` is the encrypted duplex stream to the client, `c.remotePublicKey` is its public key (32 bytes).

1. `encodedKey = z32(c.remotePublicKey)`; DEBUG `Incoming connection received from <encodedKey>`; `activeConnections[encodedKey] += 1` (a map `z32 key → counter`, for logs only).
2. `args.udp` falsy → `handleTCP`, otherwise → `handleUDP`. The branch is chosen by the server's flag value, NOT by the DHT record: `--udp=false` (a string) is truthy, UDP mode (quirk 6).
3. `handleTCP`: DEBUG `Handling TCP connection`; on the stream's `close` the counter is decremented, at zero the entry is removed and DEBUG `Disconnected from <encodedKey>` is written; then `pipeTcpServer(c, { port, host }, { isServer: true, compress: false, logger }, stats)` ([pipe.md](pipe.md)); DEBUG `TCP connection piped`.
4. `handleUDP`: DEBUG `Handling UDP connection`; the same close accounting; `pipeUdpFramedServer(c, { port, host }, logger, stats)`; DEBUG `UDP connection framed and piped`.

`this.connection` is overwritten by the result of the last call (unused). The number of parallel connections is not limited.

## The rest of the `HolesailServer` API (for the library, the CLI does not call it)

| Method | Action |
|---|---|
| `key` (getter) | private — `z32(seed)` (`seed` is the 32 bytes of sha256); public — `z32(keyPair.publicKey)` |
| `info` | `{ type:'server', state, secure, port, host, protocol:'udp'\|'tcp', seed (hex from args.seed), key, publicKey:z32(publicKey) }`; `state` — `null` → `'listening'` → `'paused'` → `'destroyed'` |
| `pause()` | INFO `Pausing server`; `dht.suspend()`; `state='paused'`; INFO `Server paused` |
| `resume()` | INFO `Resuming server`; `dht.resume()`; `state='listening'`; INFO `Server resumed` |
| `destroy()` | INFO `Destroying server`; clears the interval (INFO `Cleared DHT refresh interval`), `dht.destroy()`, nulls the fields, `state='destroyed'`, INFO `Server destroyed` |
| `put(data, opts)`, `get(opts)` | writing and reading the DHT record, [dht-records.md](dht-records.md) |

`Holesail.info` (what the banner uses): `{ type, state, secure, port, host, protocol, seed, key, url, publicKey }` — `key` for private mode is `K` itself, for public `z32(publicKey)`; `url = 'hs://' + ('s000' | '0000') + key`. `Holesail.pause()`/`resume()` delegate to the server; `Holesail._close()` calls `dht.destroy()` without waiting and resets `running`.

## Server log messages (for `--log`)

INFO: `Starting server`, `Using Private Mode`/`Using Public Mode`, `Server listening on key: <key>`. DEBUG: `Generated key pair from seed: <hex>`, `Server created, awaiting listen`, `Initializing DHT with host info: <json>`, `Putting DHT record: <json>`, `Incoming data type: string, value: <json>`, `Checking for existing DHT record`, `Existing DHT record found: seq=<n>, value=<text>`, `Existing record found, putting with seq: <n> (same: <true|false>)` or `No existing DHT record found, creating new`, `DHT put completed with seq: <n>`, `Incoming connection received from <z32>`, `Handling TCP connection`/`Handling UDP connection`, `TCP connection piped`/`UDP connection framed and piped`, `Disconnected from <z32>`, `Refreshing DHT record: <json>`; plus the piping messages ([pipe.md](pipe.md)). Logs cannot be compared byte by byte (timestamps, network ordering); comparing the set of messages is fine.

## Notes from JS runs

- **The `secure` string** (only from a library call, `new Holesail({ server: true, secure: 'false' })`; it does not come from the CLI — `!argv.public` is a boolean): `Holesail.#initialise` checks
  truthiness, so the keys are derived as for private (without `key` the seed is random), while `HolesailServer` compares strictly (`secure === true`) and runs publicly. A hybrid; Rust `secure: bool`
  cannot express it (confirmed with `testvectors/probes-js/keys.cjs`).
- **Empty seed:** `if (!seed)` — `''`, `undefined`, `null`, `0` give 32 random bytes (confirmed with `js/server-facade-dht.cjs`).
- **The secret mask (quirk 9):** a secret is everything the key pair is derived from. The seed (`Generated key pair from seed`) is secret in both modes, masked in both; the public-mode key
  (`z32(publicKey)`) is a public key, printed as is.
