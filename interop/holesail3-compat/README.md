# interop/holesail3-compat — compatibility probe holesail 3.0 ↔ 2.4.4

What it is: a one-off probe: do holesail 3.0 (branch `3.0`, `holesail-client` and
`holesail-server` 3.0.0-dev.0) and the 2.4.4 reference understand each other. The harness is separate from the `interop/run.mjs` matrix: it starts its own testnet on
127.0.0.1 (the public DHT is not touched), runs the pairs "server 3.0 ← client 2.4.4" and "server 2.4.4 ← client 3.0", stops
its own processes by PID. Success — 1000 bytes through the TCP tunnel to the local echo came back equal.

What it proves (the result is in `result.txt`; ports, keys and milliseconds change between runs): the controls
3.0↔3.0 and 2.4.4↔2.4.4 are ok; the two sides are incompatible. The 2.4.4 client does not parse the `hs_…` string of server 3.0
(`Invalid character in base32 input`), the 3.0 client does not accept the `hs://…` of server 2.4.4 (`Invalid invite format`), and at
the wire level 3.0 expects a 33-byte header (capability and mode) that 2.4.4 does not have. The decision "the reference is
2.4.4" rests on this: the probe result is in `result.txt`.

| File | What |
|---|---|
| `compat.mjs` | entry point: testnet, echo, servers and clients of both versions, scenarios S0/S0b (controls), A1–A2, B1–B3 |
| `preload3.cjs` | `node -r` preload: `TOWNJUMP_BOOTSTRAP` → `HyperDHT.BOOTSTRAP` of `hyperdht` from the 3.0 tree |
| `client3-api.mjs` | 3.0 client through the API without probe (the 3.0 CLI always sends a probe for TCP): `node -r ./preload3.cjs client3-api.mjs <invite> <port>` |
| `result.txt` | recorded result of a run |

## How to run

Requires node v24, the 2.4.4 reference in `.tmp/npm-holesail` (`interop/setup-js.sh`) and the 3.0 tree in `.tmp/holesail-3.0`
at the commit that was tested:

```
git clone --branch 3.0 https://github.com/holesail/holesail .tmp/holesail-3.0
git -C .tmp/holesail-3.0 checkout 5ae1a420d82161c142da1a1d82324f44ad67182b
(cd .tmp/holesail-3.0 && npm install)
node interop/holesail3-compat/compat.mjs
```

Process logs and a fresh `result.txt` are written to `.tmp/holesail3-compat-out/` (the directory is recreated on start).
Paths in the scripts are computed from their location.
