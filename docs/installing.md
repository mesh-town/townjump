# Installing

The command is called `townjump`. The npm package `holesail` installs a different command (`holesail`), so the two
do not collide. For every channel below, `townjump --version` prints `2.4.4 (townjump <version>)` once the install
worked.

All files are also listed on the [releases page](https://github.com/mesh-town/townjump/releases) with a
`SHA256SUMS` file. Direct downloads without version in the name are at <https://mesh-town.github.io/townjump-dist/>
(`macos/`, `linux/`, `windows/`, `android/`).

## Homebrew (macOS)

```
brew install mesh-town/macos/townjump
```

This adds the tap `mesh-town/homebrew-macos` and installs the formula in one step. The formula is for macOS only
(Apple silicon and Intel) and installs the prebuilt binary from the release; it does not build from source.
Update: `brew upgrade townjump`. Remove: `brew uninstall townjump`.

## Prebuilt binaries

Releases contain archives named `townjump-<version>-<target>.tar.gz` (`.zip` for Windows) with `townjump`, `LICENSE`
and `README.md`:

| Target | Platform |
|---|---|
| `aarch64-apple-darwin`, `x86_64-apple-darwin` | macOS |
| `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`, `armv7-unknown-linux-musleabihf`, `armv7-unknown-linux-musleabi` | Linux, static (ARM with and without FPU) |
| `mipsel-unknown-linux-musl`, `mips-unknown-linux-musl` | Linux, static; the binaries behind the OpenWrt and Keenetic feeds |
| `x86_64-pc-windows-msvc` | Windows |
| `aarch64-linux-android` | Android, for Termux |

Verify the download against `SHA256SUMS` from the same release:

```
shasum -a 256 -c SHA256SUMS --ignore-missing
```

To update, download the new archive and replace the binary; to remove, delete it.

On macOS, a downloaded unsigned binary may be quarantined by Gatekeeper; remove the attribute with
`xattr -d com.apple.quarantine townjump`.

## Windows

```
winget install MeshTown.Townjump
```

Alternatively run the installer `townjump-setup-<version>.exe` from the release; it is listed in `SHA256SUMS`. It
needs administrator rights (Windows asks for them) and 64-bit Windows. It installs `townjump.exe` into
`C:\Program Files\Townjump` and adds that directory to the system `PATH`; open a new terminal window afterwards.
`townjump-setup-<version>.exe /S` installs silently. To update, run a newer installer (or `winget upgrade
MeshTown.Townjump`). To remove, use "Apps" (Installed apps) in the Windows settings or run `uninstall.exe` in the
installation directory. The installer is not signed, so SmartScreen may show a warning: choose "More info", then
"Run anyway", after checking the file against `SHA256SUMS`.

## Debian and Ubuntu

```sh
arch="$(dpkg --print-architecture)"
curl -fLO "https://mesh-town.github.io/townjump-dist/linux/debian/townjump_${arch}.deb"
sudo apt install "./townjump_${arch}.deb"
```

Architectures: `amd64`, `arm64`, `armhf`. The package adds the apt source `/etc/apt/sources.list.d/townjump.sources`
and the signing key `/usr/share/keyrings/townjump.gpg`, so later versions arrive with
`sudo apt update && sudo apt upgrade`. Remove: `sudo apt remove townjump`. The public key is also published as
`townjump.gpg` next to the packages.

Details, removal and the build: `packaging/debian/README.md`.

## Docker

```
docker run --rm -it ghcr.io/mesh-town/townjump:latest --live 3000
```

Tags: `latest` and `<version>`; platforms linux/amd64, arm64 and arm/v7. Arguments after the image name are
`townjump` arguments. A container reaches a service on the host through the host's address, not `127.0.0.1`; use
`--host <address>` for that.

## Android (Termux)

Download `townjump-termux-aarch64.tar.gz` from the release or from
<https://mesh-town.github.io/townjump-dist/android/>, unpack it and put `townjump` into `$PREFIX/bin`:

```sh
tar -xzf townjump-termux-aarch64.tar.gz
install -m 755 townjump "$PREFIX/bin/townjump"
```

Update by repeating this with a newer archive; remove with `rm "$PREFIX/bin/townjump"`.

## OpenWrt

```sh
wget -O- https://mesh-town.github.io/townjump-openwrt/install.sh | sh
```

Works with `opkg` (24.10) and `apk` (25.12). The script fetches the feed's public key, adds the feed and installs
`townjump`; run it again to upgrade. Public keys (`townjump-usign.pub` for opkg, `townjump-apk.pem` for apk) are
served from the same address, so the feed signature can be checked by hand.

Details, architectures, removal and the build: `packaging/openwrt/README.md`.

## Keenetic

```sh
wget -O- https://mesh-town.github.io/townjump-keenetic/install.sh | sh
```

Entware must be installed first. Run the script again to upgrade.

Details, architectures, removal and the build: `packaging/keenetic/README.md`.

## From source

```
cargo install --locked --path crates/townjump
```

See [building.md](building.md).
