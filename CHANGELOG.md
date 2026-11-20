# Changelog

All notable changes are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project uses [Semantic Versioning](https://semver.org/).

## 0.1.26

- Fix: a server reached from the same public address as the client (both behind one home router, the server also
  behind a second NAT such as a virtual machine network) now sends a probe to the client's LAN addresses in every punch
  round. A neighbor on the same LAN connects directly instead of relying on the router to loop traffic back.

## 0.1.25

- Fix: a relay counts a relayed stream as opened only once both ends of the pair are wired, as JavaScript
  blind-relay does. A failure in the middle of wiring no longer leaves the opened and closed stream counters apart.
- Fix: the congestion controller's app-limited mark wraps at 2^32 delivered packets instead of overflowing.
- Homebrew formula check works with Homebrew versions that require `brew trust` for a local tap.
- More tests: relay pairing failures, stream limits on relayed connections, retransmission timeout during ProbeRTT,
  server errors before start.

## 0.1.24

- Fix: a server that accepted a connection now keeps a route back to the client, so it can open new streams to that
  client right away instead of looking the client up in the DHT and punching a new hole.
- Fix: a server whose announcement reached fewer than three relays re-announces after 3, 6, 12, 24, 48 and 60 seconds
  instead of waiting for the next full announcement five minutes later. A server started before its network is up
  becomes reachable within seconds. It keeps pinging the relays it has between these attempts.
- Fix: routes to a peer survive while any punched socket to that peer is alive; a failed attempt or a closed short
  stream no longer drops them.
- Fix: a stream that goes through a relay is closed when the relay session ends, instead of hanging.
- Library API: a server can set its relay addresses after it is created.
- Debug log lines (`peeroxide_dht::hyperdht=debug`, `peeroxide_dht::server=debug`) for handshake replies, dial
  targets and announce results; a warning when a server announcement was accepted by no relay.
- npm channel: `npm install -g townjump` (launcher package with per-platform binary packages).
- The keys and logger crates are independent implementations of a written behavioral spec.
- `CONTRIBUTING.md`: contributions are accepted under Apache-2.0.

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
