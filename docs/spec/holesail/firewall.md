# Private-mode firewall: when JS calls it and with what arguments

Sources: `holesail-server/index.js:HolesailServer.start` (the predicate), `hyperdht/lib/server.js:Server` (the `firewall` field, `_onpeerhandshake`, `_addHandshake`, `_clearLater`). This is quirk 15 in [README.md](README.md). The peeroxide fork has to provide it: peeroxide 1.7.3 has no `firewall` callback.

## Do not confuse the two "firewalls"

1. The `firewall` callback (this document): the server's decision "whether to accept a handshake from this key". Authorization.
2. The NAT-type flag `firewalled` / `FIREWALL.OPEN|UNKNOWN|CONSISTENT|RANDOM`: what a node reports about its reachability (in JS `dht.firewalled`, in peeroxide `SwarmConfig.dht.dht.firewalled`). This is about holepunching, not about access.

## What holesail passes

- Private mode: `firewall = (remotePublicKey) => !b4a.equals(remotePublicKey, keyPair.publicKey)`; true means "refuse".
- Public mode: `firewall: false` (in hyperdht `opts.firewall || (() => false)`, i.e. "let everyone through").

holesail ignores the other arguments of the call.

## The call site in hyperdht (`Server._addHandshake`)

The chain: an incoming `PEER_HANDSHAKE` request (direct or via a relay) → `_onpeerhandshake` → `_addHandshake(k, noise, clientAddress, req, direct)`, where `k = hex(noise)` is the client's Noise message; for one `k` the handshake is set up once (the `_connects` cache), repeated requests wait for the same result.

The order inside `_addHandshake`:

1. A record `hs` is reserved in `_holepunches` with `hs.firewalled = true` (the initial value is "refused").
2. `remotePayload = await handshake.recv(noise)`: parsing the client's first Noise message; from it the client's static public key (`handshake.remotePublicKey`) and the payload are extracted. A parse error → the record is cleared, no reply.
3. The server is closing or suspended (`_closing`, `suspended`) → cleanup, no reply.
4. `hs.firewalled = await this.firewall(handshake.remotePublicKey, remotePayload, clientAddress)`:
   - `remotePublicKey` — 32 bytes, the client's static key from the handshake (the client proves possession of it);
   - `remotePayload` — the client's parsed payload (version, the firewall type the client claims, addresses, udx and holepunch parameters); holesail does not read it;
   - `clientAddress` — `{ host, port }`, where the request came from (`peerAddress`, otherwise the packet sender's address).
   The callback may return a value or a promise.
5. An exception (or a rejected promise) inside the callback is caught and ignored, `hs.firewalled` stays `true` — refusal (fail closed).
6. `_closing || suspended || hs.firewalled` (a truthy value of any kind — `true`, a non-empty string, a nonzero number) → `_clearLater(hs, id, k)` (releasing the record with a delay), the function returns `null`.
7. Only after this are the server's addresses computed, the puncher created and the reply Noise message formed.

## What is visible from outside on refusal

- `_onpeerhandshake` receives `null` and returns `null`: the server does NOT reply to the request. There is no reply Noise, no puncher is created.
- The `connection` event does not fire, holesail's `onConnection` is not called, and the line `Incoming connection received from …` is absent from the server log.
- The client gets no reply: attempts through all relays end in a timeout, the result is `PEER_CONNECTION_FAILED` (per the source; not confirmed by a network measurement).
- The refusal is indistinguishable from "no server": the client is not told the reason.

## Requirement for the peeroxide fork (`peeroxide-dht`, server)

1. A callback `firewall(remote_public_key: [u8; 32], remote_payload, client_address) -> bool` (may be asynchronous), set when the server is created; by default "let everyone through". holesail 2.4.4 passes a synchronous function, so in Rust the callback is synchronous; an asynchronous variant is added later without breaking the API.
2. Called at the same point: after the client's Noise message is parsed successfully, BEFORE the reply, before the puncher is created and before any connection state is allocated, except the record that is dropped on refusal.
3. True = refuse: there is no handshake reply, no connection, nothing is reported outward. An error or panic in the callback = refusal.
4. One call per handshake (repeats of the same message do not call the callback again).

Difference from the "check after the connection" variant: a foreign peer completes the handshake, gets a working encrypted connection and therefore learns that a server exists at this address; the server spends a puncher and addresses on it. The outcome for the data is the same (the connection is closed), but it is observable from outside, so it is not a 1:1 match.

## Parity tests (for the test harness)

| # | Condition | Expectation |
|---|---|---|
| 1 | private server, client with the same `K` | a connection exists, the server logs `Incoming connection received from <z32>` |
| 2 | private server, client with a different `K` | no connection, the server has no `Incoming connection…`, the client ends with `PEER_CONNECTION_FAILED` |
| 3 | private server, public client (random pair) | as in 2 |
| 4 | public server, any client | a connection exists |
| 5 | the callback throws an exception | refusal, as in 2; the server keeps running |

## Security consequence

The server's and client's key pair in private mode is the same one and is derived from `sha256(K)`. Access is given by any of three: `K`, `sha256(K)` in hex (the seed), `z32(sha256(K))`. The INFO log prints the second and third (`Server listening on key: …`, `Connecting to key: …`, quirk 9), the server's DEBUG log prints the seed in hex (`Generated key pair from seed: …`).
