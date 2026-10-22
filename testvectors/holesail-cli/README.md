# holesail-cli — reference behavior of the holesail 2.4.4 CLI

Captured from the JS CLI without a network: `hyperdht` is replaced by `mock-dht.cjs`, which writes a trace of DHT calls.
`run-matrix.mjs` runs 95 cases and writes `cli-matrix.json` (arguments → stdout, exit code, trace);
`gen-matrix-md.mjs` makes `docs/spec/holesail/cli-matrix.md` from it, `show.mjs` prints the cases.
The absolute path of the project root in `stdout` and `stderr` (Node stacks) is recorded as `<root>` — the file does not depend on the machine.
`keyderivation-vectors.*`, `minimist-vectors.*` — vectors of key derivation and argument parsing.
What is where and how to read it — `docs/spec/holesail/README.md`, section "Reproduction artifacts".
`minimist-vectors.*` — 149 records: 114 general cases and 35 for `Object.prototype` names (all throw `TypeError`, the end of the list).
`node-port-vectors.json` — what Node 24.4 throws on a non-numeric port (`net.connect({ port })`, `listen`, unary `+`); captured by `testvectors/probes-js/jsval.cjs`.
