# Changelog

All notable changes are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project uses [Semantic Versioning](https://semver.org/).

## 0.1.x — Unreleased

First public version of the port of Holesail 2.4.4.

- CLI `townjump` (help text and messages as in the original): `--live`, `--connect` (and a bare key), `--filemanager`, `--lookup`, `--help`, `--version`;
  options `--host`, `--port`, `--udp`, `--public`, `--key`, `--log`.
- `--version` prints `2.4.4 (townjump <version>)`; the patch number grows with every merge into `main`.
- Wire- and key-compatible with JavaScript Holesail 2.4.4 (TCP and UDP, private and public keys).
- DHT layer: a patched fork of peeroxide (`fork/peeroxide`).
- Web file manager, a port of livefiles 1.1.0.
- Reference vectors and an interop test harness against JavaScript Holesail.
- `Dht::connect_direct`: connect to a peer at a known IPv4 address of its DHT socket, without bootstrap or a
  DHT lookup; `Dht::local_port`: the port of the node's DHT socket.
- `DhtConnection::remote_addr` and `DhtConnection::rtt`: the peer's address and the smoothed round-trip time
  of a connection.
- `NodeMode::firewalled`: whether the node considers itself behind a firewall; `NodeMode::default()` reports
  `firewalled: true`.
