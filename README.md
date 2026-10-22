# townjump

Share a local TCP or UDP port with another machine over a peer-to-peer network: no public IP, no port forwarding,
no VPN and no relay server. One side runs `townjump --live 3000` and gets a key; the other side runs
`townjump <key>` and the service appears on its own `127.0.0.1`.

townjump is an unofficial Rust port of [Holesail](https://github.com/holesail/holesail) 2.4.4. It speaks the same
protocol and uses the same keys, so it works together with the original JavaScript `holesail`, and it is a single
native binary that needs no Node.js.

> **Not affiliated with Holesail / LIVEPORT.** This is an independent reimplementation. "Holesail" is the name of the
> original project; it is used here only to say what this one is compatible with.

## Why a Rust port

- **One self-contained binary.** No Node.js, npm or runtime to install: copy the file and run it. Linux builds are
  statically linked (musl).
- **Small devices.** Builds for ARM routers and boards, including an [OpenWrt](https://openwrt.org/) package.
- **Drop-in compatible.** Same command-line options, the same help text, the same `hs://` keys and the same wire
  format as Holesail 2.4.4. You can run a Rust server for JavaScript clients or the other way round.
- **Comparable speed.** See [Performance](#performance).

## How it works

Holesail is built on [HyperDHT](https://github.com/holepunchto/hyperdht), the distributed hash table of the
[Holepunch](https://holepunch.to/) stack (the same network used by Hyperswarm and Keet).

1. **The server** (`--live <port>`) derives a key pair from a random or given secret and announces its public key
   in the DHT. The connection string it prints (`hs://s000…`) encodes that secret.
2. **The client** (`<key>`) derives the same public key, finds the server through DHT nodes and the two peers punch
   a hole through their NATs over UDP.
3. **The data** flows directly between the peers: an end-to-end encrypted Noise stream over
   [UDX](https://github.com/holepunchto/udx-native), a reliable transport on top of UDP. DHT nodes only help the
   peers find each other and exchange the handshake; they never see the traffic.
4. **The client** listens on a local port (`127.0.0.1:8989` by default) and forwards every TCP connection
   (or UDP datagram with `--udp`) to the server's local port.

The DHT and UDX layer is [peeroxide](https://github.com/Rightbracket/peeroxide), a Rust implementation of HyperDHT
and UDX, in a patched copy at [`fork/peeroxide`](fork/peeroxide/FORK.md). The fork adds what Holesail needs and
upstream does not have: a DHT server with a firewall callback, wire fixes that match the JavaScript reference, and
suspend/resume.

## Quick start

On the machine that has the service, say a web server on port 3000:

```
townjump --live 3000
```

It prints a connection string like `hs://s000…` (and a QR code). On another machine:

```
townjump hs://s000…
```

Open `http://127.0.0.1:8989`. Stop either side with Ctrl+C.

## Usage

```
townjump --live <port> [--host <address>] [--udp] [--public] [--key <key>]
townjump --connect <key> [--host <address>] [--port <port>] [--udp] [--public]
townjump <key>                               # same as --connect <key>
townjump --filemanager <dir> [--host <host>] [--port <port>] [--public]
            [--username <user>] [--password <pass>] [--role <admin|user>] [--safe-mode]
townjump --lookup <key>
townjump --help [filemanager]
townjump --version
```

| Option | Meaning |
|---|---|
| `--live <port>` | Share the local port `<port>` (server). |
| `--connect <key>`, `<key>` | Connect to a server and expose it on a local port (client). |
| `--host <address>` | Server: the local address to forward to (default `127.0.0.1`). Client: the address to listen on. |
| `--port <port>` | Client: the local port to listen on (default `8989`, or the one the server advertises). |
| `--udp` | Forward UDP instead of TCP. Both sides must use it. |
| `--key <key>` | Server: use a fixed secret instead of a random one, so the connection string survives restarts. |
| `--public` | Use a public key (see below). |
| `--lookup <key>` | Print what the server advertises in the DHT, without connecting. |
| `--log [level]` | Enable the debug log (levels 0 to 3). |

Run `townjump --help` for the full text. The help and all messages are byte-for-byte those of the original,
so they say `holesail`, not `townjump`.

### Private and public keys

- **Private** (`hs://s000…`, the default): the string is the secret. Anyone who has it can connect, so treat it like
  an SSH key. The server rejects clients that do not know it.
- **Public** (`hs://0000…`, `--public`): the string is the public key; it can be published, and anyone can connect.
  Protect the service itself (for example with a password).

### File manager

`--filemanager` serves a directory as a web file manager (a port of
[livefiles](https://github.com/holesail/livefiles) 1.1.0) and shares it at the same time:

```
townjump --filemanager ./shared --username alice --password 'a long password' --safe-mode
```

The server prints a connection string; the other side connects with `townjump <key>` and opens the page in a
browser. The page lists directories, downloads files and creates new folders and empty files; there are no uploads,
deletions or renames. Only the `admin` role (`--role admin`) can create folders. Without `--username` and `--password`
the login is `admin` / `admin`.

`--safe-mode` is an extension of this port (not in the original). It refuses to start without a non-default password
and hardens the server: only `GET` and `POST` are accepted, `POST` from another origin and requests with an
unexpected `Host` header are refused (which also blocks DNS-rebinding attacks), symbolic links that point outside
the shared directory are hidden and refused, existing files are not overwritten, and listing pages are
HTML-escaped. With `--safe-mode`, the page is reachable by IP address or a single-label or `.local` name only,
not through a reverse proxy with a public domain name. The full rules are in
[docs/spec/holesail/filemanager.md](docs/spec/holesail/filemanager.md).

### Environment variables

| Variable | Meaning |
|---|---|
| `TOWNJUMP_BOOTSTRAP` | Comma-separated `host:port` list of DHT bootstrap nodes, for a private DHT or a test network. Unset or empty: the public HyperDHT bootstrap nodes. |
| `RUST_LOG` | Enables the internal `tracing` log of the DHT and transport layers (for example `RUST_LOG=info`), separate from `--log`. |

### Windows

`townjump.exe` runs from PowerShell or `cmd`. The first time you start a server (`--live` or `--filemanager`),
Windows Defender Firewall asks whether `townjump.exe` may accept incoming connections. Allow it: a server
that was not allowed stays unreachable (clients time out, the server keeps running). If the dialog was closed or
never shown, allow the program in Windows Security, Firewall & network protection, Allow an app through firewall,
or from an elevated PowerShell:

```
New-NetFirewallRule -DisplayName townjump -Direction Inbound -Program C:\path\to\townjump.exe -Action Allow
```

A client (`townjump <key>`) only connects out and needs no rule.

## Differences from the original Holesail

The goal is to behave exactly like JavaScript Holesail 2.4.4. Where this port differs, the difference is listed here.

### Compatible

- **Wire and keys.** A Rust server and client interoperate with JavaScript `holesail` 2.4.4 in all four combinations,
  over TCP and UDP, with private and public keys. The test suite checks this against a local DHT test network and
  against reference vectors captured from the JavaScript implementation.
- **Command line.** The same commands and options, the same help text and messages, the same exit codes. Even the
  quirks of the original are kept, so that scripts written for `holesail` keep working: for example
  `--public=false` still means public, and `--live P --port Q` ignores `Q`. The list is in
  [docs/spec/holesail/README.md](docs/spec/holesail/README.md).
- **`--version`** prints `2.4.4`, the version of Holesail this port follows, not the version of the port.

### Different

- **Command name.** The binary is `townjump`, so it can be installed next to the npm `holesail`.
- **Secrets in logs.** With `--log`, the original prints the private secret in full. This port prints its first
  8 characters and replaces the rest with `*`.
- **File manager.** `--safe-mode` (above) is new. Even without it, paths that escape the shared directory
  (`..`, `%2e%2e`) are refused with `403`; form fields with multi-byte characters are decoded correctly; the user name
  and password are compared in constant time.
- **UDP on Windows.** Forwarding with `--udp` does not stop when the remote side is briefly unreachable
  (ICMP "port unreachable"), and datagrams up to 64 KiB are accepted.
- **Faster shutdown.** Ctrl+C on a server withdraws its DHT announcement in parallel with a 1.5 s limit; the server
  exits in under a second instead of several seconds. A record that could not be withdrawn expires on its own.
- **Network changes.** The node's public address follows the last 16 observations, so after switching networks
  (for example Wi-Fi to LTE) it moves to the new address without a restart, after a few minutes of DHT traffic.
- **Same public IP.** A client behind the same public IP as a Rust server connects without hole punching.

### Not supported

- **Holesail 3.x.** Version 3.0 changed the connection string and the wire header; this port speaks 2.4.4 only.
- **32-bit MIPS** (many older routers): the code needs 64-bit atomics.
- **Automatic reconnection** of the client is not implemented, as in the original.
- **Network interface events.** After a network change the node learns its new address from DHT traffic only,
  not from the operating system, so this takes minutes.
- **Persistent DHT node.** A long-running node does not become a full DHT node (as hyperdht does after about
  20 minutes behind a friendly NAT) and does not save its routing table between runs.

### Performance

Measured over a 220–250 ms round-trip internet path, plain UDX streams:

| Direction | townjump | holesail (JS) |
|---|---|---|
| Upload | 78–86 Mbit/s | 72–75 Mbit/s |
| Download | 30–37 Mbit/s | 30–37 Mbit/s (the path's limit) |

On a single machine (loopback, 64 MiB transfers) the port moves 28–32 MiB/s against 10–17 MiB/s for JavaScript.
Opening a new connection takes about the same time as JavaScript on a repeat connection (about 1.5 s on such a
path); the first, cold connection is slower (2.6–3.1 s against 1.9–2.6 s). Under heavy packet loss the port is
currently much slower than JavaScript; this is being worked on.

## Install

Release 0.1.0 is not published yet; the commands below describe the prepared channels.

- **Prebuilt binaries:** download the archive for your platform from the
  [releases page](https://github.com/mesh-town/townjump/releases) and verify it against `SHA256SUMS`.
  Targets: macOS (Apple silicon, Intel), Linux static (x86_64, aarch64, armv7), Windows (x86_64).
- **Homebrew (macOS, Linux):** `brew tap mesh-town/meshtown && brew install townjump`
- **OpenWrt:** build the package from `packaging/openwrt/townjump` with the OpenWrt SDK or as a feed.
- **From source:** `cargo install --locked --path crates/townjump` (Rust 1.94 or newer; see
  [docs/building.md](docs/building.md)).

Details for every channel: [docs/installing.md](docs/installing.md).

## Using it as a library

The crate `townjump` exposes the `Holesail` type, a port of the JavaScript `Holesail` class: `new`, `ready`, `info`,
`pause`, `resume`, `close` and `Holesail::lookup`. The DHT is passed in as a factory (`PeeroxideFactory` from
`townjump-dht-peeroxide`), so the facade can run on a mock network in tests. The crates are not published on
crates.io yet and the API is not stable; depend on them through git.

## Crates

| Crate | Purpose |
|---|---|
| `townjump` | The `townjump` command: CLI and the `Holesail` facade (the crate is named `townjump`). |
| `townjump-client` | Client side: connects to a server by key and exposes it on a local port. |
| `townjump-server` | Server side: publishes a local port in the DHT and proxies incoming connections to it. |
| `townjump-keys` | Keys: z-base-32, seed from a string, `hs://` parsing, server and client key pairs. |
| `townjump-jsval` | JavaScript value semantics needed for argv and DHT records (`Number#toString`, JSON, port validation). |
| `townjump-pipe` | Stream splicing: TCP and framed UDP. |
| `townjump-logger` | The debug log and the channel for unhandled errors. |
| `townjump-dht` | Traits of the DHT network: the seam between the Holesail layer and an implementation (peeroxide or a mock). |
| `townjump-dht-peeroxide` | Adapter of `townjump-dht` over the peeroxide fork; the only crate that sees the fork. |
| `townjump-filemanager` | The `--filemanager` web file manager (port of livefiles 1.1.0). |
| `townjump-testkit` | Test support: reference-vector loader, mock DHT, shared conformance suite (dev-only). |

`fork/peeroxide` is a vendored, patched copy of [peeroxide](https://github.com/Rightbracket/peeroxide) 1.7.3
(the list of changes is in [FORK.md](fork/peeroxide/FORK.md)); it is a separate Cargo workspace.

## Documentation

- [docs/building.md](docs/building.md) — build from source and cross-compile.
- [docs/installing.md](docs/installing.md) — install channels.
- [docs/testing.md](docs/testing.md) — running the tests, including the interop tests against JavaScript Holesail.
- [docs/spec/](docs/spec/) — the behaviour specification of the port, with the JavaScript sources it follows.
- [CHANGELOG.md](CHANGELOG.md), [SECURITY.md](SECURITY.md), [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

## Credits

- [Holesail](https://github.com/holesail/holesail) and [livefiles](https://github.com/holesail/livefiles) — the
  original programs this port reproduces.
- [Holepunch](https://github.com/holepunchto): HyperDHT, UDX, secret-stream — the protocol stack.
- [peeroxide](https://github.com/Rightbracket/peeroxide) — the Rust implementation of HyperDHT and UDX this port
  is built on.

## License

[AGPL-3.0-only](LICENSE). `fork/peeroxide` keeps its own licenses (MIT OR Apache-2.0, see the license files in the
fork); the logo and file-manager page resources come from livefiles 1.1.0 (GPLv3). See
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
