# interop/ — compatibility test harness Rust ↔ JS

Runs the real holesail CLIs (the JS 2.4.4 reference and the Rust binary) against each other on a **local testnet** of hyperdht
on 127.0.0.1 and prints "scenario → ok/fail". The public DHT is not touched: both sides get only the testnet address as
bootstrap, from the `TOWNJUMP_BOOTSTRAP` variable.

Terms. **testnet** — a bootstrap node and 4 ordinary DHT nodes in one node process. **A `client→server` pair** —
which implementation sits on which side: `js→js`, `js→rust`, `rust→js`, `rust→rust`. **Echo** — a service on 127.0.0.1
that returns everything it received (TCP and UDP); the holesail server forwards traffic to it, the test talks to
the local port of the holesail client. **Private mode** — link `hs://s000<K>`, the server admits only a client with
the same K; **public** — `hs://0000<z32-key>`, admits everyone.

## Running

```
node interop/run.mjs                              # the whole matrix; without a Rust binary the rust pairs are skipped
node interop/run.mjs --client js --server js      # JS↔JS only (about 30 seconds)
node interop/run.mjs --rust-bin <path>            # Rust binary (or TOWNJUMP_BIN=<path>)
node interop/run.mjs --only udp,fw --client js    # some scenarios (id substring; --list shows the list)
node interop/run.mjs --help
```

Requires node v24 and the JS reference tree: `.tmp/npm-holesail/node_modules` (installed by `interop/setup-js.sh`;
another location — `TOWNJUMP_JS_NM`). Exit code: 0 — no fail, 1 — some fail, 2 — startup error (no reference,
testnet did not start); `--strict` counts skip as a failure too.

Output (time in seconds; the lines without data below are an example too):

```
harness: client→server; testnet 127.0.0.1:60260 (4 nodes); JS …/node_modules; Rust none; logs …/.tmp/interop-out
ok    js→js     tcp-private              1.9s
FAIL  js→js     udp-public               30.4s
      UDP :51579: returned 0 of 20 datagrams in 30000 ms
          [js server] …last lines of the process output…
skip  js→rust   (9 scenarios)           no Rust binary: --rust-bin or TOWNJUMP_BIN
total: 9 ok, 0 fail, 27 skip
```

Logs of every process — `.tmp/interop-out/<client>-to-<server>.<scenario>.{server,client}.log` (the directory
is recreated on every run). Two runs in a row give the same result: ports are fresh each time, keys are random,
all processes are stopped at the end (and on Ctrl-C).

## Scenarios

Each runs with any pair; the client connects by the `hs://…` link from the server banner, `--udp` is not passed to the client
(the protocol is taken from the DHT record).

| id | what it checks |
|---|---|
| `tcp-private` | private server (`--key <phrase>`), 1 MiB through the echo, bytes equal, then the client closes its half — the echo answers EOF; the server has exactly one `Incoming connection received from` line |
| `tcp-public` | the same on a public server (`--public`) |
| `udp-private`, `udp-public` | 20 numbered datagrams of 5…2048 bytes (`--udp` on the server), all returned intact; one incoming connection |
| `udp-truncate` | a 4000-byte datagram (`--udp`, private server): the application gets one datagram of the first 2048 bytes, the remainder does not arrive separately; then a control datagram passes (`docs/spec/holesail/pipe.md`, "UDP: the frame") |
| `tcp-private-two-conn` | two TCP connections in a row through one client (one identity): 1000 bytes, then 1 MiB; the server sees two incoming |
| `tcp-private-big` | 8 MiB through the tunnel, bytes equal |
| `fw2-wrong-key` | a client with a foreign key K′: the local connection is closed without data, the server has no `Incoming…` line; then a client with its own key passes |
| `fw3-public-client` | a public client with the public key of a private server (random identity): firewall rejection — as in `fw2`; then the own client passes |
| `tcp-private-nat` | **only via `--only tcp-private-nat`**: JS client with `remoteAddress() = null` (NAT emulation, holepunch path); fails in 3–5 of 12 runs even on JS↔JS, so it is not in the matrix |

`fw2` and `fw3` are tests 2 and 3 of the firewall test table (`docs/spec/holesail/firewall.md`). Only `fw3` really tests
the firewall: for `fw2` there is no DHT record under the derived key K′, and the client gets `PEER_NOT_FOUND` within
milliseconds. Self-check of the harness: `node interop/run.mjs --client js --server js --only fw --mutate open-firewall`
(the JS server admits everyone) — `fw3-public-client` must give FAIL.

## How Rust is plugged in

The Rust side is the same CLI with the same argv (`--live`, `--key`, `--public`, `--udp`, `--port`, `--log`, the link
as a positional argument) and the variable `TOWNJUMP_BOOTSTRAP=host:port[,host:port…]`. The binary is looked up as:
`--rust-bin`, `TOWNJUMP_BIN`, then `$CARGO_TARGET_DIR/{release,debug}/<name>` and
`target/{release,debug}/<name>`, where `<name>` is the `name` of the first `[[bin]]` in `crates/townjump/Cargo.toml` (`townjump`); it is run directly or through the wrapper from `TOWNJUMP_RUN_WRAPPER`. The harness reads the CLI output:
the banner (`hs://…`), the lines `Server listening on key`, `Proxy listening on 127.0.0.1:<port>`, the DEBUG line
`Incoming connection received from` (the server is started with `--log 0`, the client with `--log 1`) — the format is as in JS
(`docs/spec/holesail/server.md`, "Server log messages").

To check the Rust-side launch path without a Rust binary: `node interop/run.mjs --rust-bin interop/js/js-as-rust.cjs` —
a fake "binary" runs the JS CLI inside; all 4 pairs must be ok.

## Files

| File | What |
|---|---|
| `run.mjs` | entry point: flag parsing, testnet, loop over pairs and scenarios, printing the total |
| `js/testnet.mjs` | testnet: `node interop/js/testnet.mjs [N] [--exit-on-stdin-eof]`; the first stdout line is `TOWNJUMP_BOOTSTRAP=127.0.0.1:<port>`; lives until SIGTERM/SIGINT (with the flag — also until stdin closes). `JsTestnet::start` from `townjump-testkit` calls it the same way |
| `js/bootstrap-env.cjs` | `node -r` preload: `TOWNJUMP_BOOTSTRAP` → `HyperDHT.BOOTSTRAP` (the JS CLI has no option of its own); `INTEROP_JS_FIREWALL=unknown`, `INTEROP_JS_MUTATE=open-firewall` — switches of the harness itself |
| `js/echo.mjs` | TCP+UDP echo; a module (`startEcho()`) and a program (`node interop/js/echo.mjs [tcp] [udp]`) |
| `js/nm.cjs` | where the reference npm tree is (`TOWNJUMP_JS_NM`) |
| `js/js-as-rust.cjs` | fake "Rust binary" for checking the harness |
| `lib/scenarios.mjs`, `lib/world.mjs` | scenarios; the scenario environment (echo, server, clients, cleanup) |
| `lib/checks.mjs` | data exchange: TCP echo, UDP echo, "no tunnel" |
| `lib/impl.mjs`, `lib/proc.mjs`, `lib/testnet.mjs`, `lib/ports.mjs`, `lib/keys.mjs`, `lib/banner.mjs`, `lib/args.mjs` | CLI launch, child processes, testnet, free ports, server key from K, banner parsing, flags |

A compatibility probe with holesail 3.0 is a separate harness, not part of the matrix: `interop/holesail3-compat/README.md`.

Helper tests: `node --test 'interop/lib/*.test.mjs'` (including the `udpTruncated` self-check on a fake proxy).

## What the harness does not do

The public network (the same scenarios without the testnet); a Rust node as a DHT node for JS; several simultaneous TCP
or UDP senders; empty datagrams (datagrams over 2048 bytes — scenario `udp-truncate`);
`--filemanager`, `--lookup`. Note: on localhost the client is always OPEN (see `INTEROP_JS_FIREWALL` in `interop/js/bootstrap-env.cjs`).
