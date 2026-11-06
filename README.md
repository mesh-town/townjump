# townjump

Share a local TCP or UDP port with another machine through NAT, with no public IP, no port forwarding and no server
of your own. One side runs `townjump --live 3000` and gets a key; the other runs `townjump <key>` and the service
appears on its own `127.0.0.1`.

townjump is an independent Rust port of [Holesail](https://github.com/holesail/holesail) 2.4.4: the same keys, the
same wire protocol and the same command line, in one static binary that needs no Node.js. It runs on macOS, Windows,
Linux, OpenWrt and Keenetic routers and in Termux.

> **Not affiliated with Holesail or LIVEPORT.** "Holesail" is the name of the original project and is used here only
> to say what this one is compatible with.

[Mesh Town](https://mesh-town.github.io/), a private network of your own devices, is built on townjump.
Website: <https://mesh-town.github.io/townjump/>

[Quick start](#quick-start) | [Install](#install) | [Usage](#usage) | [Compatibility](#compatibility-with-holesail) |
[Library](#using-it-as-a-library) | [Documentation](#documentation) | [License](#license)

## Quick start

On the machine that has the service, say a web server on port 3000:

```
townjump --live 3000
```

It prints a connection string like `hs://s000...` (and a QR code). On another machine:

```
townjump hs://s000...
```

Open `http://127.0.0.1:8989`. Stop either side with Ctrl+C. The connection string is a secret: whoever has it can
reach your service.

## Install

| System | Command or file |
|---|---|
| macOS | `brew install mesh-town/macos/townjump`, or an archive from the [release](https://github.com/mesh-town/townjump/releases/latest) |
| Windows | `winget install MeshTown.Townjump`, or `townjump-setup-<version>.exe` from the release |
| Linux | static archives for x86_64, aarch64, armv7 and MIPS from the release |
| Debian, Ubuntu | `townjump_<arch>.deb` from <https://mesh-town.github.io/townjump-dist/linux/debian/>, then `sudo apt install ./townjump_<arch>.deb`; updates come with `apt upgrade` |
| OpenWrt 24.10 / 25.12 | `wget -O- https://mesh-town.github.io/townjump-openwrt/install.sh \| sh` |
| Keenetic (Entware) | `wget -O- https://mesh-town.github.io/townjump-keenetic/install.sh \| sh` |
| Docker | `docker run --rm -it ghcr.io/mesh-town/townjump:latest --live 3000` |
| Android (Termux) | `townjump-termux-aarch64.tar.gz` from <https://mesh-town.github.io/townjump-dist/android/> |
| From source | `cargo install --locked --path crates/townjump` (Rust 1.94+) |

Verify downloads against `SHA256SUMS` of the release. All channels, updates and removal:
[docs/installing.md](docs/installing.md).

## Why a Rust port

- **One self-contained binary.** No Node.js, npm or runtime to install: copy the file and run it. Linux builds are
  statically linked (musl).
- **Small devices.** Static musl builds for ARM and MIPS routers; packages for OpenWrt and Keenetic.
- **Drop-in compatible.** Same command-line options, the same help text, the same `hs://` keys and the same wire
  format as Holesail 2.4.4. You can run a Rust server for JavaScript clients or the other way round.
- **Extras.** Local network discovery, saved DHT nodes and a hardened file manager mode; see
  [docs/usage.md](docs/usage.md).

## How it works

Holesail is built on [HyperDHT](https://github.com/holepunchto/hyperdht), the distributed hash table of the
[Holepunch](https://holepunch.to/) stack (the same network used by Hyperswarm and Keet).

1. **The server** (`--live <port>`) derives a key pair from a random or given secret and announces its public key
   in the DHT. The connection string it prints (`hs://s000...`) encodes that secret.
2. **The client** (`<key>`) derives the same public key, finds the server through DHT nodes and the two peers punch
   a hole through their NATs over UDP.
3. **The data** flows between the peers: an end-to-end encrypted Noise stream over
   [UDX](https://github.com/holepunchto/udx-native), a reliable transport on top of UDP. DHT nodes only help the
   peers find each other and exchange the handshake; they never see the traffic. If the hole punch fails, the stream
   goes through a blind relay, which sees only encrypted bytes.
4. **The client** listens on a local port (`127.0.0.1:8989` by default) and forwards every TCP connection
   (or UDP datagram with `--udp`) to the server's local port.

The DHT and transport layer is a patched copy of peeroxide; see [docs/library.md](docs/library.md).

## Usage

```
townjump --live <port> [--host <address>] [--udp] [--public] [--key <key>]
townjump --connect <key> [--host <address>] [--port <port>] [--udp] [--public]
townjump <key>                               # same as --connect <key>
townjump --filemanager <dir> [--username <user>] [--password <pass>] [--role <admin|user>] [--safe-mode]
townjump --lookup <key>
townjump --help [filemanager] | --version
```

| Option | Meaning |
|---|---|
| `--live <port>` | Share the local port `<port>` (server). |
| `--connect <key>`, `<key>` | Connect to a server and expose it on a local port (client). |
| `--host <address>` | Server: the local address to forward to (default `127.0.0.1`). Client: the address to listen on. |
| `--port <port>` | Client: the local port to listen on (default `8989`, or the one the server advertises). |
| `--udp` | Forward UDP instead of TCP. Both sides must use it. |
| `--key <key>` | Server: use a fixed secret, so the connection string survives restarts. |
| `--public` | Use a public key (see below). |
| `--lookup <key>` | Print what the server advertises in the DHT, without connecting. |
| `--log [level]` | Enable the debug log (levels 0 to 3). |
| `--safe-mode` | File manager: hardened mode (townjump only). |
| `--save-nodes <file>` | Keep the DHT routing table between runs (townjump only). |

Run `townjump --help` for the full text. The help and all messages are byte-for-byte those of Holesail, so they say
`holesail`. Options that exist only in townjump are marked in [docs/usage.md](docs/usage.md): `--safe-mode` for the
file manager, `--save-nodes` to keep the DHT routing table between runs, and local network discovery.

### Private and public keys

- **Private** (`hs://s000...`, the default): the string is the secret. Anyone who has it can connect, so treat it like
  an SSH key. The server rejects clients that do not know it.
- **Public** (`hs://0000...`, `--public`): the string is the public key; it can be published, and anyone can connect.
  Protect the service itself (for example with a password).

### File manager

`townjump --filemanager ./shared --safe-mode --password '<long password>'` serves a directory as a web file manager
and shares it; the other side opens the page in a browser. Details: [docs/usage.md](docs/usage.md).

## Compatibility with Holesail

Keys and wire: a townjump server or client works with JavaScript `holesail` 2.4.4 in all four combinations, over TCP
and UDP, with private and public keys. Command line: the same options, help text and exit codes, including the quirks
of the original, so scripts written for `holesail` keep working. `--version` prints `2.4.4 (townjump <version>)`.
Not supported: Holesail 3.x (a different connection string and wire header). What differs and what is measured:
[docs/compatibility.md](docs/compatibility.md).

## Using it as a library

The crate `townjump` exposes the `Holesail` type, a port of the JavaScript `Holesail` class. The DHT is passed in as
a factory, so the facade can run on a mock network in tests. The crates are not on crates.io and the API is not
stable; depend on them through git. Crates, relay options and an example: [docs/library.md](docs/library.md).

## Documentation

- [docs/usage.md](docs/usage.md) - commands, options, keys, file manager, environment variables, firewalls.
- [docs/installing.md](docs/installing.md) - install channels, updates, removal, signatures.
- [docs/compatibility.md](docs/compatibility.md) - what is the same, what differs, what is measured.
- [docs/library.md](docs/library.md) - crates and the Rust API.
- [docs/building.md](docs/building.md) - build from source and cross-compile.
- [docs/testing.md](docs/testing.md) - running the tests, including interop tests against JavaScript Holesail.
- [docs/spec/](docs/spec/) - the behaviour specification, for contributors.
- [CHANGELOG.md](CHANGELOG.md), [SECURITY.md](SECURITY.md), [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

## Credits

- [Holesail](https://github.com/holesail/holesail) and [livefiles](https://github.com/holesail/livefiles) - the
  original programs this port reproduces.
- [Holepunch](https://github.com/holepunchto): HyperDHT, UDX, secret-stream - the protocol stack.
- [peeroxide](https://github.com/Rightbracket/peeroxide) - the Rust implementation of HyperDHT and UDX this port
  is built on.

## License

[AGPL-3.0-only](LICENSE). `fork/peeroxide` keeps its own licenses (MIT OR Apache-2.0, see the license files in the
fork); the logo and file-manager page resources come from livefiles 1.1.0 (GPLv3). See
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
