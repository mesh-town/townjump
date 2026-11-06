# Changelog

All notable changes are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project uses [Semantic Versioning](https://semver.org/).

## 0.1.20 - first public release

First public version of the port of Holesail 2.4.4.

- CLI `townjump` (help text and messages as in the original): `--live`, `--connect` (and a bare key), `--filemanager`,
  `--lookup`, `--help`, `--version`; options `--host`, `--port`, `--udp`, `--public`, `--key`, `--force`, `--log`.
- `--version` prints `2.4.4 (townjump <version>)`; the patch number grows with every merge into `main`.
- Wire- and key-compatible with JavaScript Holesail 2.4.4 (TCP and UDP, private and public keys).
- Web file manager, a port of livefiles 1.1.0, with the hardened `--safe-mode` (townjump only).
- `--save-nodes`, `--save-nodes-ttl`, `--save-nodes-every`: keep the DHT routing table between runs (townjump only).
- Local network discovery over IPv4 multicast, off with `TOWNJUMP_LAN_DISCOVERY=0` (townjump only).
- DHT layer: a patched fork of peeroxide (`fork/peeroxide`).
- Library API: `ServerOpts` / `ConnectOpts` relay options (`relay_through`, `relay_addresses`, `holepunch`),
  `DhtConnection::relay_path`, `Dht::connect_direct` and `Dht::local_port`, `DhtConnection::remote_addr` and
  `DhtConnection::rtt`, `NodeMode::firewalled`.
- Reference vectors and an interop test harness against JavaScript Holesail.
- Release archives for macOS, Linux (x86_64, aarch64, ARMv7, MIPS, MIPSEL), Windows and Termux; Homebrew, winget,
  Debian, OpenWrt, Keenetic and Docker channels.
