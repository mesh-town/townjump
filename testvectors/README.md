# testvectors — reference vectors from the pinned JS

The vectors are captured by the Node scripts in `testvectors/gen/` on the JS tree of these versions
(holesail 2.4.4, hyperdht 6.34.0, dht-rpc 6.27.0, udx-native 1.21.3, …). Rust tests read the JSON and do not run JS.

**Regeneration:** `node testvectors/gen/all.mjs` (about 90 seconds; network — only 127.0.0.1).
**Determinism check:** two runs into two copies of the directory and `git diff --no-index` — empty (no randomness and no time; timings are rounded to 100 ms, tolerance ±50 ms). The ±50 ms tolerance for timings is a fixed convention, not an open question: Rust tests compare moments with that tolerance, and timings are written to the files rounded to 100 ms or not written at all (only boolean outcomes of series). A full run twice into two copies of the directory (`.tmp/` with symlinks to the module tree) gave byte-identical `testvectors/` directories (second pass). The module tree is looked up relative to the script location (`gen/lib.mjs`: `.tmp/npm-holesail/node_modules`).

| Layer | File | What | Script |
|---|---|---|---|
| holesail | `holesail/url-parser.json` | `Holesail.urlParser`, 49 connection strings | `gen/holesail-keys.mjs` |
| holesail | `holesail/key-derivation.json` | server/client × key × secure: seed, key pair, public key, `key` string, z32 errors (84) | `gen/holesail-keys.mjs` |
| holesail | `holesail/dht-record-signing.json` | mutable signature (`signMutable`), seq, put/get encodings, negative `verifyMutable` | `gen/holesail-records.mjs` |
| holesail | `holesail/dht-record-live.json` | live run on testnet 127.0.0.1: put/get of a holesail record, seq 0,0,1,2,3,3, signature from the network | `gen/holesail-records.mjs` |
| holesail | `holesail-cli/keyderivation-vectors*`, `minimist-vectors*` | captured earlier, read as is; `key-derivation.json` is checked against them in the script | (old) |
| hyperdht | `hyperdht/messages.json` | handshake, noisePayload, holepunch, holepunchPayload, peer(s), lookupRawReply, announce | `gen/wire-codecs.mjs` |
| dht-rpc | `dht-rpc/header.json` | request and response header: flags, tid, to, id, token, closerNodes, error, value | `gen/wire-codecs.mjs` |
| version difference | `deltas/codec.json` | D1 NS_PLUGIN and pluginRequest, D12 c.uint/c.buffer (new and old), D13 protomux batch frames | `gen/deltas-codec.mjs` |
| version difference | `deltas/plugin.json` | D1: PING, PLUGIN (10), unknown command 11 against a testnet node without `id` and a persistent node with `id`, hyperdht 6.34.0 (`new`) and 6.30.0 (`old`): without `id` both give `UNKNOWN_COMMAND`; with `id` the new one is silent (plugin `x` answers), the old one gives `UNKNOWN_COMMAND` | `gen/deltas-plugin.mjs` |
| version difference | `deltas/udx.json` | D2 D3 empty MESSAGE and HEARTBEAT, D4 zero-window probes, D6 strict SACK, D7 RTO, D8 ACK for every DATA — raw socket against an udx-native stream 1.21.3 (`new`) and 1.19.2 (`old`) | `gen/deltas-udx.mjs` |
| version difference | `deltas/udx-probes.json` | D7 TLP (20 attempts per version, boolean outcome), D9 MTU probe (10 probes, `data_offset`, lengths), D9 `change_remote` and `changeRemoteInflight` | `gen/deltas-udx-probes.mjs` (shared test rig `gen/udx-rig.mjs`) |
| version difference | `deltas/blind-relay.json` | D14: unpair in blind-relay 1.6.1 (`new`) and 1.4.0 (`old`) — which streams of the pair are closed: from the initiator, from the responder, on session drop | `gen/deltas-blind-relay.mjs` |
| version difference | `deltas/handshake-socket.json` | D11: whether the server announces the holepuncher socket in the handshake response (loopback, hyperdht 6.34.0 and 6.30.0) — neither version announces it | `gen/deltas-handshake-socket.mjs` |
| secret-stream | `secret-stream/xx-inband.json` | `@hyperswarm/secret-stream` 6.9.2, Noise XX "in-band": keys and ephemeral keys are fixed, 2 Noise messages (`noiseOutputs`), `handshakeHash`, stream header (id + 24 bytes of the secretstream header), 18 wire records (`wire`), data frames of different lengths, unordered `send`, an empty keepalive message, repeated chunk splitting | `gen/secret-stream.mjs` |
| secret-stream | `secret-stream/ik-hyperdht.json` | the same for Noise IK as hyperdht creates it (`start(raw, {handshake})`): 8 Noise messages, 15 wire records | `gen/secret-stream.mjs` |
| secret-stream | `secret-stream/negative.json` | 10 negative receive cases (wrong header length, corrupted frame, etc.) | `gen/secret-stream.mjs` |

`deltas-js/` — the earlier one-off probes D1–D8 (read only); `deltas/` — their repeatable replacement.

How secret-stream was captured without editing foreign code: the Noise ephemeral key (`noise.e`) is set before the first send; `crypto_secretstream_init_push` in the secret-stream module is replaced by "header = BLAKE2b-24(key) + the regular `init_pull`" (push and pull states by header are equal); the stream's `randombytes_buf` is deterministic. Frame bodies of 65536 and 70000 bytes — only length and sha256.

Not captured (reasons): D7 — the exact interval to the probe packet `1·2 + 2` ms (the new version's retransmission arrives within <20 ms, only a boolean outcome goes to the file: the old one without retransmission has 6–8 of 20 attempts); D9 — the `seq` position of MTU probes (30 versus 29 between runs), the libudx be51b3a difference in `changeRemoteInflight` did not reproduce; D11 — the source port of the reply when the client is on the same LAN (the loopback testnet does not distinguish the branch, a node with a private address is needed); a real exchange through the blind relay over UDX and the scenario "relay → direct".
