# Testing

## Default tests

```
cargo test --workspace --locked
```

They need no network beyond `127.0.0.1` and no Node.js. They use a mock DHT and the reference vectors committed in
`testvectors/` (captured from the JavaScript implementation). The fork has its own tests:

```
cd fork/peeroxide && cargo test --workspace
```

Tests that need the JavaScript side are marked `#[ignore]` and run only on request.

## Interop tests against JavaScript Holesail (`--ignored`)

Requirements: Node.js 24 and a copy of JavaScript Holesail 2.4.4 with its dependencies.

```
interop/setup-js.sh                     # creates .tmp/npm-holesail (npm install holesail@2.4.4 --ignore-scripts)
cargo test -p townjump-dht-peeroxide -- --ignored
cargo test -p townjump-testkit -- --ignored
```

Each such test starts a JavaScript DHT test network (a bootstrap node plus four nodes on `127.0.0.1`); the public DHT is not
touched. To use dependencies installed elsewhere, set `TOWNJUMP_JS_NM` to the `node_modules` directory:

```
TOWNJUMP_JS_NM=/path/to/node_modules cargo test --workspace -- --ignored
```

## Scenario matrix: JavaScript and Rust CLIs against each other

```
cargo build --release -p townjump
node interop/run.mjs --rust-bin target/release/townjump     # all pairs: js/rust client x js/rust server
node interop/run.mjs --client js --server js                # one pair
node interop/run.mjs --list                                 # scenarios
```

Output is `ok`/`FAIL`/`skip` per scenario; exit code 0 means no failures. Logs go to `.tmp/interop-out/`.
See `interop/README.md` for details.

## Fork tests that compare with Node.js

Some tests in `fork/peeroxide` compare Rust output with a Node.js reference. Install its dependencies first:

```
cd fork/peeroxide/tests/node && npm ci --ignore-scripts
```

## Regenerating reference vectors

```
node testvectors/gen/all.mjs        # about 90 seconds; network only on 127.0.0.1
```

Needs the same JavaScript tree as above. The run is deterministic: two runs give byte-identical files.

## Running binaries through a wrapper

If your system requires a wrapper to start freshly built binaries, set the `TOWNJUMP_RUN_WRAPPER` variable to the
wrapper command; without it, binaries are started directly.
