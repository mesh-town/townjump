# Server record in the DHT: publishing, reading, `--lookup`

Sources: `holesail-server/index.js:HolesailServer.put`, `HolesailServer.get`, `holesail-client/index.js:HolesailClient.get`, `HolesailClient.ping`, `holesail/src/index.js:Holesail.lookup`, the layer below — `hyperdht/index.js:HyperDHT.mutablePut`, `HyperDHT.mutableGet`. Based on reading the code; behavior on a live network has not been verified.

## What is stored in the DHT

One mutable record per server, signed with the server's Ed25519 key (`keyPair`, [connection-string.md](connection-string.md)):

- Address in the DHT: `target = blake2b-256(publicKey)` (32 bytes, `crypto_generichash` without a key) — records are looked up by it on the nearest nodes. The signature covers `(seq, value)`; the hyperdht layer signs and verifies (`Persistent.signMutable`, `Persistent.verifyMutable`), holesail does not call them.
- `seq` is an unsigned integer that grows with every change of the value.
- `value` is the UTF-8 bytes of the string `JSON.stringify({ host, udp, port })` in THIS key order; keys with the value `undefined` do not reach the JSON. The values are as they came from the flags, without type coercion: `host` is the string of the server's target address (default `127.0.0.1`), `port` is a number from minimist, `udp` is `true`, `undefined` (no key), the string `'false'`, a number, etc. (quirk 5, quirk 6). Example: `{"host":"127.0.0.1","udp":true,"port":3000}`; without `--udp`: `{"host":"127.0.0.1","port":3000}`. For `--filemanager` — the filemanager's port and its host.
- Size limit: the comment in the `put` code says 1 KB; the real check is done by the lower layer (not verified).
- The record contains NEITHER the key NOR the secret `K`; in private mode the record address is derived from `sha256(K)`, so only someone who knows `K` can find the record.

## Publishing (`HolesailServer.put(data)`)

`data` is the string above, `null`/`undefined` → `Error('data cannot be undefined')`. The order:

1. DEBUG `Putting DHT record: <data>` and `Incoming data type: string, value: <data>`.
2. `data = Buffer.from(data)`. DEBUG `Checking for existing DHT record`.
3. `old = await get({ latest: true })` — a read by its OWN public key; a found record is logged at DEBUG `Existing DHT record found: seq=<n>, value=<text>`.
4. Choosing `seq`:
   - no record → `seq` is unset, `mutablePut` substitutes `0` (`opts.seq || 0`); DEBUG `No existing DHT record found, creating new`;
   - a record exists, `old.value == null` → `old.seq + 1` (DEBUG `oldRecord.value is null or undefined`);
   - a record exists, the value bytes are EQUAL to the new ones → `seq = old.seq` (the same version, the record is simply republished), otherwise `old.seq + 1`; DEBUG `Existing record found, putting with seq: <n> (same: <true|false>)`.
5. `mutablePut(keyPair, data, { seq })`: a walk over the nearest nodes with a `MUTABLE_GET` request (value `0`), then to every node that answered a `MUTABLE_PUT` request with the signed `{ publicKey, seq, value, signature }` and the node's token. Returns `{ publicKey, closestNodes, seq, signature }`; DEBUG `DHT put completed with seq: <n>`.
6. `put` does not track network errors: a request with no responding nodes completes silently (per the source; not confirmed by a live check).

Schedule: the first `put` is inside `start`, before the banner (quirk 8); then every 50 minutes (`50 * 60 * 1000` ms) a `put` with the SAME value (that is, with the same `seq` if the read sees the record), DEBUG `Refreshing DHT record: <data>`. An error inside the timer is not caught (an unhandled rejection → code 1).

## Reading (client)

`HolesailClient.get(opts)` → `dht.mutableGet(publicKey, opts)`, where `publicKey` is the public key of the CLIENT pair ([client.md](client.md)): in private mode it matches the server's key, in public mode it comes from the `z32.decode` key.

The behavior of `mutableGet` in hyperdht 6.34.0: `latest = opts.latest !== false`, i.e. by default (`{}` and `{ latest: true }` are the same) the walk covers ALL nodes that answered, replies with a wrong signature and with `seq` below `opts.seq || 0` are discarded, and of the rest the record with the highest `seq` is taken. The value is a Buffer, holesail converts it to a UTF-8 string. Empty → `null`.

A requirement for the peeroxide fork: `mutable_get(pk, seq)` in peeroxide 1.7.3 returns the FIRST verified reply: when replicas diverge it returns a stale record. A default `latest` mode (as in JS) is required at least for the client read and the read before `put`, otherwise the "read, then `seq + 1`" scheme may understate `seq`.

## `--lookup` (`Holesail.lookup` → `HolesailClient.ping`)

Output and codes — [cli.md](cli.md) (the `--lookup` section), quirk 14. The algorithm:

1. `urlParser(url)` → `{ key, secure }` ([connection-string.md](connection-string.md)).
2. `secure` truthy → `argKey = z32(sha256(key))`. Otherwise `z32.decode(key)` for validation; an error → `Error('Invalid key format: <key>')`.
3. `ping(argKey)` (a new `HyperDHT` with a random identity, destroyed afterwards): `keyBuffer = z32.decode(argKey)`.
   - Attempt A "as private": `publicKey = HyperDHT.keyPair(keyBuffer).publicKey` (`keyBuffer` is taken as a seed — a length of 32 bytes is needed, otherwise an exception), `mutableGet(publicKey, { latest: true })`.
   - If there is no result — attempt B "as public": `publicKey = keyBuffer`, the same request.
   - In each attempt the value is parsed with `JSON.parse`; invalid JSON is silently skipped (the attempt counts as empty); on success `result.protocol = result.udp ? 'udp' : 'tcp'`.
     Non-obvious (verified by `testvectors/probes-js/client.cjs`): `result = JSON.parse(...)` comes BEFORE the assignment of `protocol`, so for a `null` record the assignment fails inside the `try` and the attempt is empty, while a primitive record (`5`, `true`, `"str"`; `0`, `""`, `false` — if there is no second attempt) is returned by `ping` as is; arrays (`[]`, `[1]`) are returned as arrays, `{}` gives `{"protocol":"tcp"}`. `Holesail.lookup` on a primitive record throws `TypeError` (`result.secure = …` in the class's strict mode). `LookupRecord` must hold any JSON value, and the `--ping` output must match JS byte for byte.
   - Result: the parsed record with `protocol` added, or `null`.
4. `lookup`: `result = ping(...) || {}`, then `result.secure = secure` (`true` or `undefined`), i.e. the "Private" mark in the output is the mark from the URL, NOT what attempt A or B found.
5. The CLI prints the fields `host`, `port`, `protocol`, `secure`; a missing record yields `{}` (an empty result, the "No record found…" branch does not fire, see [cli.md](cli.md)); any exception — `Error during lookup:` and code 0.

## What the client does with the record

Merging `port`/`host`/`udp` with the flags — [client.md](client.md) (quirk 11). The record is used ONLY by the client and `--lookup`: the server does not read it when accepting connections.
