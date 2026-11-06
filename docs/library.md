# Using townjump as a library

The crates are not published on crates.io and the API is not stable; depend on them through git and pin a tag:

```toml
[dependencies]
townjump = { git = "https://github.com/mesh-town/townjump", tag = "v0.1.20" }
```

The crate `townjump` exposes the `Holesail` type, a port of the JavaScript `Holesail` class: `new`, `ready`, `info`,
`pause`, `resume`, `close` and `Holesail::lookup`. The DHT is passed in as a factory (`PeeroxideFactory` from
`townjump-dht-peeroxide`), so the facade can run on a mock network in tests.

## Crates

| Crate | Purpose |
|---|---|
| `townjump` | The `townjump` command: CLI and the `Holesail` facade. |
| `townjump-client` | Client side: connects to a server by key and exposes it on a local port. |
| `townjump-server` | Server side: publishes a local port in the DHT and proxies incoming connections to it. |
| `townjump-keys` | Keys: z-base-32, seed from a string, `hs://` parsing, server and client key pairs. |
| `townjump-jsval` | JavaScript value semantics needed for argv and DHT records (`Number#toString`, JSON, port validation). |
| `townjump-pipe` | Stream splicing: TCP and framed UDP. |
| `townjump-logger` | The debug log and the channel for unhandled errors. |
| `townjump-dht` | DHT network traits: the seam between the Holesail layer and an implementation (peeroxide or a mock). |
| `townjump-dht-peeroxide` | Adapter of `townjump-dht` over the peeroxide fork; the only crate that sees the fork. |
| `townjump-lan` | Local network discovery of servers over IPv4 multicast. |
| `townjump-filemanager` | The `--filemanager` web file manager (port of livefiles 1.1.0). |
| `townjump-testkit` | Test support: reference-vector loader, mock DHT, shared conformance suite (dev-only). |

## DHT extensions

Beyond the JavaScript API, `townjump-dht` offers:

- **Blind relay.** `ServerOpts` and `ConnectOpts` have `relay_through` (a `RelayThrough`: one relay key or a list
  to try in turn), `relay_addresses` (relay keys with addresses, reached without a DHT lookup) and a `holepunch`
  filter. `DhtConnection::relay_path` reports whether the data currently flows through a relay or directly.
- **Direct connect.** `Dht::connect_direct` connects to a peer at a known IPv4 address of its DHT socket, without
  bootstrap or a DHT lookup; `Dht::local_port` is the port of the node's own DHT socket.
- **Connection details.** `DhtConnection::remote_addr` and `DhtConnection::rtt` give the peer's address and the
  smoothed round-trip time.
- **Node mode.** `NodeMode::firewalled` tells whether the node considers itself behind a firewall.

## The peeroxide fork

The DHT and transport layer is a patched copy of [peeroxide](https://github.com/Rightbracket/peeroxide) 1.7.3, a
Rust implementation of HyperDHT and UDX, in `fork/peeroxide`. It is a separate Cargo workspace pulled in by path
dependencies. The fork adds what Holesail needs and upstream does not have: a DHT server with a firewall callback,
wire fixes that match the JavaScript reference, and suspend/resume. The change list is in
[FORK.md](../fork/peeroxide/FORK.md).
