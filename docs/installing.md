# Installing

The command is called `townjump`. The npm package `holesail` installs a different command (`holesail`), so the two
do not collide. The help text of `townjump` is byte-for-byte that of the original and says `holesail`.

Release 0.1.0 is not published yet; the channels below are prepared.

## Homebrew (macOS)

```
brew install mesh-town/macos/townjump
```

This adds the tap `mesh-town/homebrew-macos` and installs the formula in one step. The formula is for macOS only
(Apple silicon and Intel) and installs the prebuilt binary from the release; it does not build from source.

## Prebuilt binaries

Releases (`https://github.com/mesh-town/townjump/releases`) contain archives named
`townjump-<version>-<target>.tar.gz` (`.zip` for Windows) with `townjump`, `LICENSE` and `README.md`:

| Target | Platform |
|---|---|
| `aarch64-apple-darwin`, `x86_64-apple-darwin` | macOS |
| `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`, `armv7-unknown-linux-musleabihf`, `armv7-unknown-linux-musleabi`, `mipsel-unknown-linux-musl`, `mips-unknown-linux-musl` | Linux, static |
| `x86_64-pc-windows-msvc` | Windows |

Verify the download against `SHA256SUMS` from the same release:

```
shasum -a 256 -c SHA256SUMS --ignore-missing
```

On macOS, a downloaded unsigned binary may be quarantined; remove the attribute with
`xattr -d com.apple.quarantine townjump`.

## Windows

The installer `townjump-setup-<version>.exe` is among the release files and is listed in `SHA256SUMS`. It needs
administrator rights (Windows asks for them) and 64-bit Windows. It installs `townjump.exe` into
`C:\Program Files\Townjump` and adds that directory to the system `PATH`; open a new terminal window afterwards.
`townjump-setup-<version>.exe /S` installs silently. To remove it, use "Apps" (Installed apps) in the Windows settings
or run `uninstall.exe` in the installation directory. The installer is not signed, so SmartScreen may show a warning.

A winget package will follow after the first release; this page will show the command once the package exists.

## Debian and Ubuntu

```sh
arch="$(dpkg --print-architecture)"
curl -fLO https://mesh-town.github.io/townjump/apt/townjump_${arch}.deb
sudo apt install ./townjump_${arch}.deb      # later: sudo apt update && sudo apt upgrade
```

The feeds are not published yet and the address is provisional until the first publication.

Details, architectures, removal and the build: `packaging/debian/README.md`.

## OpenWrt

```sh
wget -O- https://mesh-town.github.io/townjump/openwrt/install.sh | sh        # opkg (24.10) and apk (25.12); run again to upgrade
```

Details, architectures, removal and the build: `packaging/openwrt/README.md`.

## Keenetic

```sh
wget -O- https://mesh-town.github.io/townjump/keenetic/install.sh | sh       # Entware must be installed first
```

Details, architectures, removal and the build: `packaging/keenetic/README.md`.

## From source

```
cargo install --locked --path crates/townjump
```

See [building.md](building.md).
