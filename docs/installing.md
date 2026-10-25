# Installing

The command is called `townjump`. The npm package `holesail` installs a different command (`holesail`), so the two
do not collide. The help text of `townjump` is byte-for-byte that of the original and says `holesail`.

Release 0.1.0 is not published yet; the channels below are prepared.

## Homebrew

```
brew tap mesh-town/tap
brew install townjump
```

The formula builds from the release source archive with `cargo install`.

## Prebuilt binaries

Releases (`https://github.com/mesh-town/townjump/releases`) contain archives named
`townjump-<version>-<target>.tar.gz` (`.zip` for Windows) with `townjump`, `LICENSE` and `README.md`:

| Target | Platform |
|---|---|
| `aarch64-apple-darwin`, `x86_64-apple-darwin` | macOS |
| `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`, `armv7-unknown-linux-musleabihf` | Linux, static |
| `x86_64-pc-windows-msvc` | Windows |

Verify the download against `SHA256SUMS` from the same release:

```
shasum -a 256 -c SHA256SUMS --ignore-missing
```

On macOS, a downloaded unsigned binary may be quarantined; remove the attribute with
`xattr -d com.apple.quarantine townjump`.

## OpenWrt

The package definition is `packaging/openwrt/townjump/Makefile`. Build it with the OpenWrt SDK
(copy the directory into your `package/` tree or add the repository as a feed) and install the resulting `.ipk`/`.apk`.
It is not in the official `openwrt/packages` feed. Supported targets are those with 64-bit atomics (no 32-bit MIPS).
The package needs a Rust toolchain on the build host; check the Rust version required by `rust-version` in `Cargo.toml`.

## From source

```
cargo install --locked --path crates/townjump
```

See [building.md](building.md).
