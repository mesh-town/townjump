# peeroxide (fork)

Patched copy of two crates from [peeroxide](https://github.com/Rightbracket/peeroxide), a Rust implementation of the
Hyperswarm network stack that is wire-compatible with the Node.js one. Origin and our changes are in [FORK.md](FORK.md).

```
peeroxide-dht  — Kademlia DHT, Noise handshakes, hole-punching, relay (HyperDHT)
└── libudx     — reliable UDP transport with BBR congestion control (libudx)
```

- **peeroxide-dht** — HyperDHT implementation including Kademlia, hole-punching and Noise handshakes.
- **libudx** — pure Rust implementation of the UDX protocol with BBR congestion control.

This is a separate Cargo workspace; the root workspace of this repository uses both crates through path dependencies.

## Tests

```sh
cd tests/node && npm ci --ignore-scripts   # Node.js reference implementations for the interop tests
cd ../.. && cargo test --workspace
```

The interop tests compare Rust output with the Node.js reference (`tests/node/`) and with golden fixtures
(`tests/interop/`). The stack uses pure Rust crypto (RustCrypto) and has no C dependencies.

## Reference implementations

- [hyperdht](https://github.com/holepunchto/hyperdht)
- [dht-rpc](https://github.com/holepunchto/dht-rpc)
- [hyperswarm-secret-stream](https://github.com/holepunchto/hyperswarm-secret-stream)
- [libudx](https://github.com/holepunchto/libudx)

## License

Dual-licensed under `MIT OR Apache-2.0` (`LICENSE-MIT`, `LICENSE-APACHE`). See `THIRD_PARTY_NOTICES` for upstream
license details.
