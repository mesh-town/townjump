# testvectors/probes-js — probes of the JS reference (contract clarifications; facts for the peeroxide fork)

What it is: six short scripts (the first table) that run the real `holesail@2.4.4` (and its `hyperdht` facade)
and print JSON with what the library does on edge inputs where the spec and a guess diverged. `*.out.json` — the output
captured once; the "Clarifications" sections of the spec `docs/spec/holesail/` rest on it. Five more `fork-*` probes
(the second table, below) capture the behavior of the lower JS layers — `hyperdht`, `dht-rpc`, `udx-native` — for the peeroxide fork tasks.

How to run (Node and a downloaded reference are needed: `interop/setup-js.sh` puts the packages into `.tmp/npm-holesail`):

    node testvectors/probes-js/<name>.cjs        # JSON goes to stdout, errors to stderr

Package paths are computed from the script location (`__dirname/../../.tmp/npm-holesail/node_modules`), so it can be run
from any directory. No network is needed: the DHT is fake or points to the closed `127.0.0.1:1`.
The output can be compared with `*.out.json` byte for byte everywhere except two nondeterministic places: the port number in `pipe`
and milliseconds (`newHyperDHT_ms`, `destroy_ms`) in `server-facade-dht`.

| File | What it proves | What rests on it |
|---|---|---|
| `keys.cjs` | `new Holesail({ server: true, secure: 'false' })` (a string) — a hybrid: keys are derived as for private, the server works publicly | `docs/spec/holesail/server.md` ("The `secure` string" in "Notes from running JS"), test `server_keys_tests.rs` |
| `jsval.cjs` | `??` for `undefined`/`null`; what Node throws on a non-numeric port (`TypeError`, code `ERR_INVALID_ARG_TYPE`) | `testvectors/holesail-cli/node-port-vectors.json`, tests `port_tests.rs`, `record_tests.rs` (crate `townjump-jsval`) |
| `pipe.cjs` | UDP/TCP proxy: what closes and how (`end` versus `destroy`), sizes in `Data from … bytes` (a chunk with headers versus a datagram), `EADDRINUSE` on a busy port | `docs/spec/holesail/pipe.md` ("Clarifications"), tests `crates/holesail-pipe` |
| `client.cjs` | `info` before `connect` throws `TypeError`; `ping` on a `null` record; the format of the `Retrieved DHT data` line; primitive records are returned as is | `docs/spec/holesail/dht-records.md`, `client.md`; tests `client_tests.rs`, `ping_tests.rs` |
| `ready-badkey.cjs` | key not in z32: `new Holesail` does not throw, `ready()` rejects with `Invalid character in base32 input` | `docs/spec/holesail/connection-string.md`, `client.md`; test `facade_tests.rs` |
| `server-facade-dht.cjs` | `generateKeyPair` of `''`, `undefined`, `null`, `0` gives 32 random bytes; `new HyperDHT` is synchronous | `docs/spec/holesail/server.md` ("Empty seed" in "Notes from running JS"), tests `dht_tests.rs`, `server_tests.rs` |

## Probes for the peeroxide fork (`fork-*`, figures as observed)

All run without a network: the script itself starts a testnet (a local DHT network) on 127.0.0.1; the reference is the same as above
(`interop/setup-js.sh`, directory `.tmp/npm-holesail/node_modules`). A different `node_modules` directory is set by a variable:
`NM` for `fork-connect-*`, `TOWNJUMP_JS_NM` for the others. The output is lines for human reading (`PROBE <what>: <value>` and so on),
the probe checks nothing; run the same way: `node testvectors/probes-js/<name>`.

| File | What it proves (run and time) | What rests on it |
|---|---|---|
| `fork-suspend-probe-suspend.cjs` | what JS `dht.suspend()`/`resume()` does: an established tunnel is dropped, the remote peer gets `ECONNRESET` in 1–4 ms; a suspended node's own `connect` — `SUSPENDED` in 0–2 ms; `ping` of a suspended node — `REQUEST_TIMEOUT` ≈ 4 s; `findPeer` of its key — 0 responses, a foreign `connect` — `PEER_NOT_FOUND` ≈ 8 s; the port after `resume` is the same; double `suspend`/`resume` without errors (≈ 20 s) | fork tests `rpc_suspend_tests.rs`, `server/freeze_tests.rs` |
| `fork-connect-notfound-probe-notfound.cjs` | which code `connect` returns for a key nobody serves: no holders (nodes with the server record) — `PEER_NOT_FOUND` in 3–10 ms; the holder refused or is stopped — `PEER_CONNECTION_FAILED` ≈ 8 s; the holder is alive — open ≈ 8 ms (four scenarios A–D in a row, ≈ 22 s) | fork test `peeroxide-dht/tests/hyperdht_connect_notfound.rs` |
| `fork-connect-parallel-probe-parallel.cjs` | how many holders `connect` tries at once (two) and how long until a silent one drops off (≈ 4 s; all silent — 8 s and `PEER_CONNECTION_FAILED`); a holder with an error frees its slot in 1–4 ms. The argument is the case key (`a1`, `aN`, `b`, `be`, `b2`, `c`, `ce`); without an argument — all, ≈ 15 s | fork test `peeroxide-dht/tests/hyperdht_connect_parallel.rs` |
| `fork-d11-probe-js.mjs` (output — `fork-d11-probe-js.out.txt`) | which socket the JS server answers the handshake from (direct, relay, second — for an open server, fw without LAN, fw with LAN); the argument is a substring of the scenario name; ports are random, compare by the lines «arrived on» / «sends from» | `docs/spec/deltas.md` (D11), fork `handshake_route.rs` |
| `fork-throughput-probe-udx.cjs` | speed of JS `udx-native` on loopback: 100–140 MiB/s (8 MiB in 16 KiB chunks in 0.06–0.09 s); arguments `[MiB=8] [chunk=16384]`, timeout — `PROBE_TMO_MS`; one line `PROBE udx-native bytes=… secs=… mibs=… chunk=…` | fork speed tests `libudx/tests/throughput.rs`, `peeroxide-dht/tests/secret_stream_throughput.rs` |
