# Specification of the holesail 2.4.4 layer on top of HyperDHT

What the Rust version must reproduce 1:1: the `holesail` CLI, the `holesail-server` and `holesail-client` libraries, bridging TCP/UDP to a DHT stream, the private/public modes, the `hs://…` connection string, the metadata record in the DHT, the firewall behavior. Everything below the layer "DHT node, Noise handshake, UDX stream, holepunching, mutable records as a network operation" belongs to the peeroxide fork and is not described here (except one place where holesail requires a function from it — [firewall.md](firewall.md)).

This document is self-contained: the rules are written out in full, and code citations look like `package/file:symbol` (no line numbers, since they drift). The source of truth is the package sources in `.tmp/npm-holesail/node_modules/<package>` at the versions below, plus the captured CLI behavior ([cli-matrix.md](cli-matrix.md)).

## Versions captured

| Package | Version | Role |
|---|---|---|
| holesail | 2.4.4 | CLI (`src/bin/holesail.mjs`), the `Holesail` class (`src/index.js`), help text, banner, input validation |
| holesail-server | 2.4.0 | server: listens on a key pair in the DHT, puts the record, glues incoming connections to a local port |
| holesail-client | 2.3.0 | client: reads the record, listens on a local port, glues it to the outgoing DHT connection; `ping` |
| @holesail/hyper-cmd-lib-net | 1.1.2 | stream piping: `connPiper`, `createTcpProxy`, `pipeTcpServer`, UDP framing |
| holesail-logger | 1.1.0 | `--log` log lines |
| livefiles | 1.1.0 | HTTP filemanager behind `--filemanager` (see [filemanager.md](filemanager.md)) |
| hyperdht | 6.34.0 | needed only for the firewall hook ([firewall.md](firewall.md)) |
| minimist | 1.2.8 | argv parsing without options |
| barely-colours 1.0.0, cli-box 6.0.10, qrcode-terminal 0.12.0 | | ANSI colors, banner frame, QR |

## Contents

| File | Contents |
|---|---|
| [cli.md](cli.md) | flags, argv parsing, order of checks and dispatch, error texts, exit codes, banner, help text, `--lookup`, `--log` |
| [connection-string.md](connection-string.md) | the `hs://…` format, parsing, key pair derivation (server and client), private/public modes, vectors |
| [server.md](server.md) | `holesail-server`: startup, order of DHT calls, the firewall predicate, connection accounting, pause, destruction |
| [client.md](client.md) | `holesail-client`: constructor, `connect`, the local listen address, `ping`, destruction |
| [pipe.md](pipe.md) | TCP piping and framed UDP (frame format, close semantics, counters) |
| [dht-records.md](dht-records.md) | the metadata record: format, `seq`, update, client read and `lookup` |
| [firewall.md](firewall.md) | when and with what data JS calls the firewall and what happens on rejection — the requirement the fork has to meet |
| [filemanager.md](filemanager.md) | the CLI-visible surface of `livefiles`, port deviations and the "JS function → Rust file" map |
| [cli-matrix.md](cli-matrix.md), `golden/` | the "arguments → output / exit code / side effect" matrix (95 cases, for parity tests) and the reference stdout of the help texts and version |

## Reproduction artifacts (`testvectors/holesail-cli/`)

- `mock-dht.cjs` — a `node -r` preloader: replaces `hyperdht` with a stub and writes a trace of DHT calls to the file `$MOCK_TRACE`; the network is not opened. Controls: `MOCK_RANDOM_HEX` (replaces `randomBytes`), `MOCK_RECORD_FOR` (`*` or the hex of a public key) and `MOCK_RECORD` (the DHT record text), `MOCK_FAKE_CONN` (imitates an incoming connection).
- `run-matrix.mjs` — runs ~100 cases `[id, args, {env, wait, pre}]`, waits for output to go quiet, sends SIGINT, writes `cli-matrix.json` (exact stdout/stderr bytes with ANSI, trace, code). `node run-matrix.mjs <id prefix>` — a subset into `cli-matrix.part.json`.
- `cli-matrix.json` — the captured reference; `show.mjs` — a compact viewer; `gen-matrix-md.mjs` — builds [cli-matrix.md](cli-matrix.md).
- `minimist-vectors.json` (+ `minimist-vectors.mjs`) — 149 vectors "argv → minimist 1.2.8 result" (114 general cases and 35 — 12 `Object.prototype` names × 3 flag forms).
- `keyderivation-vectors.json` (+ `.mjs`) — vectors "key → seed → public key → URL".
- Running the comparison against JS: `node -r <path>/mock-dht.cjs .tmp/npm-holesail/node_modules/holesail/src/bin/holesail.mjs <args>` from a directory with `holesail` installed (in the project — `.tmp/npm-holesail/node_modules/holesail`).

## JS quirks reproduced as is

The quirks are numbered; the other files refer to them by number. Quirks 1–8 and 10–14 are reproduced 1:1 and marked in code and tests; quirk 9 is an intentional deviation: the first 8 characters of the secret, the rest as asterisks.

| № | Quirk | Where described |
|---|---|---|
| 1 | On the client `--public` is inverted: the value goes to `secure`, so `--public` on the client turns PRIVATE mode on; without the flag a bare 64-hex key without a prefix is read as public, `z32.decode` fails (code 1) | connection-string.md, cli.md |
| 2 | The `secure` flag in `urlParser` is "the 6th character of the string equals `s`", regardless of the `hs://` prefix: a bare public z32 key whose 6th character is `s` (about 3% of keys) is read as private | connection-string.md |
| 3 | `-v` is not handled (only `--version`), it falls through to the general help with code 0; any unknown flag also gives the help, code 0 | cli.md |
| 4 | `--key` without a value: the message "Error: Key can not be empty" and code 0 (`process.exit()` without a number) | cli.md |
| 5 | minimist turns number-like values into numbers: `--key 1234…` (digits only), `--connect 00443214`, `--password 0123`, `--live 0x10`, `3000.5` — are distorted or accepted; the key becomes `String(Number(value))` | cli.md, connection-string.md |
| 6 | `--public=false` and `--udp=false` give the string `'false'` — which is truthy: the server is public, UDP is on | cli.md |
| 7 | `--live P --port Q`: port `Q` is ignored, the server forwards to `P` | cli.md, server.md |
| 8 | The banner is printed right after `ready()`: on the client — before the local proxy started listening; on the server — after the record is written to the DHT (`put`), not after `listen` | cli.md, client.md, server.md |
| 9 | With `--log 1` and below the private server prints `Server listening on key: <z32(sha256(K))>` — a value equal to the secret (the key pair is built from it); the client does the same with the line `Connecting to key: <z32(sha256(K))>`, and the server's DEBUG log prints the seed in hex. **Intentional deviation: the first 8 characters of the secret, the rest as asterisks** | server.md, client.md, firewall.md |
| 10 | `--key hs://0000…` on the server still gives private mode (the `secure` flag from the URL is only ever `true` or "unset") | connection-string.md |
| 11 | The client's default local listen address is taken from the server's DHT record (`host`/`port`), 8989 only if there is no record; the help text says "default: 8989" | client.md |
| 12 | Unhandled errors (`JSON.parse` on someone else's record, the client port in use after the banner, an invalid z32) — code 1 and a Node stack in stderr | cli.md |
| 13 | Numbers are accepted without bounds checks: `--live 99999`, `3000.5`, `--port 0` pass, `--live 0` — falls into the help (falsy) | cli.md |
| 14 | A `--lookup` error gives code 0 (errors are only printed) | cli.md |
| 15 | The JS private server rejects foreign keys at the handshake (firewall); peeroxide 1.7.3 has no such callback, so the fork adds one; private mode is not covered by an end-to-end run | firewall.md |

## Not verified by running

Without a network, the layer was verified by reading the code only: the real timings of `listen`/`connect`, real data transfer, behavior on disconnect, filemanager behavior over HTTP. An end-to-end run (JS server ← Rust client, and Rust server ← JS client, on peeroxide) covers the basic exchange; private mode and UDP are not covered by it. Anything marked "per the source" in the text is not confirmed by running.
