# Building from source

## Requirements

- Rust toolchain pinned in `rust-toolchain.toml` (installed automatically by `rustup`; `rustfmt` and `clippy` included).
- Node.js is **not** needed to build or to run the default tests (see [testing.md](testing.md)).

## Build

```
cargo build --release --locked -p townjump
./target/release/townjump --version
```

The workspace is `crates/*`. `fork/peeroxide` is a separate Cargo workspace pulled in by path dependencies
(`libudx`, `peeroxide-dht`); it is built together with the main one and needs no separate step.

## Install into `~/.cargo/bin`

```
cargo install --locked --path crates/townjump
```

The crate is named `townjump`; the installed command is `townjump`.

## Checks used in CI

```
cargo fmt --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

## Cross-compiling

Release binaries for Linux (musl) are built with [`cargo-zigbuild`](https://github.com/rust-cross/cargo-zigbuild);
see `.github/workflows/release.yml` for the exact targets and commands. 32-bit MIPS targets are not supported:
the code uses `AtomicU64`.
