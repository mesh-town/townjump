# peeroxide fork

A plain directory of this repository (no nested `.git`), a vendored copy of upstream with the changes listed below.

## Origin

- Upstream: https://github.com/Rightbracket/peeroxide
- Version: tag `peeroxide-v1.7.3`, commit `6ec917806480d769c16dfbf44a8d80413fcc0418` (`6ec91780`, "chore: release (#29)").
- The baseline is the tree of that commit without its `.git`. The upstream `.github/` directory is dropped
  (release automation that does not run inside a subdirectory).

## What we change

Our crates take only `peeroxide-dht` and `libudx` from here (path dependencies of the root `Cargo.toml`; the root has
`exclude = ["fork"]`, the fork has its own `[workspace]`). Only these two crates are carried over; upstream's
`peeroxide` and `peeroxide-cli` crates, its `docs/`, the Docker test rig and the Node scripts that only they used are not.
Changes F1-F13:

| № | Change | Where |
|---|---|---|
| F1 | clippy `drain_collect` (3 places) fixed with `std::mem::take`. | `libudx`, `peeroxide-dht` |
| F2 | DHT server: `listen` (registers the server, announces `hash(pk)` and repeats it), incoming `PeerConnection`s, several per key, `close` withdraws the announce; the announce stores `msg.to` instead of `reply.from`. | `peeroxide-dht/src/server/`, `hyperdht/announce.rs` |
| F3 | Firewall callback for the F2 server, called after the client's Noise `recv` and before the reply; public accessor `NoiseWrap::remote_public_key`. | `server/`, `noise_wrap.rs` |
| F4 | `mutable_get` returns the record with the greatest `seq`. | `hyperdht/records.rs` |
| F5 | UDX wire fixes: HEARTBEAT timer removed (reply to 0x20 kept), dead-peer timeout and close behaviour as in the JS reference, SACK handled with `data_offset = 0` on receive and send. | `libudx/src/native/stream/` |
| F6 | DHT wire fixes: plugin namespace constant and codec as in the JS reference, `UNKNOWN_COMMAND` reply to unknown commands. | `peeroxide-dht` |
| F8 | `AsyncRead + AsyncWrite` for `SecretStream`; a frame goes out in one `poll_write`; dropping `UdxAsyncStream` sends DESTROY like `destroy()` in JS; `poll_write` does not wait for the previous ACK, it accepts data while unacknowledged bytes stay below a threshold. | `secret_stream/io.rs`, `libudx/src/native/async_stream/` |
| F9 | `suspend`/`resume` of the DHT, matching `dht.suspend()`/`dht.resume()` of hyperdht 6.34.0; after `suspend` the node serves no requests, after `resume` the F2 server accepts again. | `peeroxide-dht` (`hyperdht_suspend.rs`, `rpc_suspend.rs`, `server/freeze.rs`) |
| F10 | Adaptive mode of the DHT node, matching dht-rpc 6.27.0: with `ephemeral` unset and a bootstrap list the node starts ephemeral and, after `stable_ticks` quiet ticks (default 240), turns itself persistent (`DhtHandle::mode`, `watch_mode`, `set_adaptive`). Needed for it: the response event says whether it arrived on the server socket (D5); an incoming `PING_NAT` is answered to `from.host:port-from-value` instead of `from` (D6); a promoted node publishes the verified window address with `FIREWALL_OPEN` (D7; JS `remoteAddress` returns it for any non-firewalled node, the fork only after promotion); `set_adaptive(false)` demotes a node that promoted itself (D10, JS reads the flag only in the constructor); `wake_up` demotion also restores `firewalled` from the config (JS leaves it `false`, a TODO there). | `peeroxide-dht` (`rpc/adaptive.rs`, `rpc/nat_check.rs`, `rpc/node_adaptive.rs`, `rpc/handle_mode.rs`, `rpc/node_nat.rs`, `io/`) |
| F11 | Relay-to-direct upgrade as in hyperdht 6.34.0 (`connect.js`, `server.js`) with blind-relay 1.6.1: `relay_through`, `relay_keep_alive` and `holepunch` options on `ConnectOpts`/`ServerOpts`; both sides pair through a blind-relay node, run the hole punch in parallel and move the stream to the direct path as soon as it works (libudx: `StreamControl::{change_remote, try_send_message, await_direct, set_path_hook}`, MESSAGE packets outside the sequence space); the relay fast path keeps a `Weak` link between paired streams (no `Arc` cycle) and counts `streams_opened/closed/errors`; a relay session slot is released when the peer never opens the control channel (`BlindRelaySession::release`); a responder stream holds its first packets until the firewall hook gives it the peer address (otherwise they would go to `0.0.0.0:0` and cost one RTO per pairing). | `peeroxide-dht` (`hyperdht/relay/`, `hyperdht/connect_relayed.rs`, `server/relay.rs`, `relay_service/`), `libudx/src/native/stream/` |
| F12 | Routing table seeding and export (dht-rpc `opts.nodes` / `toArray()`): `DhtConfig::nodes`, `DhtHandle::to_array`; differences in docs/spec/dht-rpc/saved-nodes.md | `peeroxide-dht` (`rpc/node_seeds.rs`) |
| F13 | Closed-stream linger (TIME_WAIT): when a stream's processor ends, its table entry stays as a tombstone for clamp(14·rto, 20 s, 60 s); late DATA/END from the recorded peer address get a DESTROY, or an ACK once our END is acknowledged and the peer's END is in order; at most one reply per rto, other addresses and packet types get nothing. udx.c removes the entry at once and leaves TIME_WAIT as a TODO (udx.c:442). | libudx/src/native/stream/linger.rs |

(The number F7 is unused.) F11 differs from the JS reference in three places, all wire-compatible:

- The `holepunch` filter is also applied in the two early `Connected` branches of `connect_holepunch.rs` (`gate_punch`), not only inside the punch round as in JS. It only acts when `holepunch` is set (the default `None` keeps the upstream behaviour). Without it an early `Connected` always wins the race on loopback and `HolepunchAborted`/`RelayAborted` could never be reached.
- The relay pairing token is made per handshake (`RelayAsk::for_handshake`), while JS makes one `relayToken` per connect (`connect.js`). The client runs handshakes through several DHT nodes at once, each with its own Noise; the server opens a slot per Noise, so with one shared token the relay paired the client with the wrong slot (`StreamIdMismatch`).
- The server's path hook ignores packets from the relay's own port when the source is a loopback alias of the relay address: a relay bound to all interfaces answers from loopback a peer that dialled its LAN address, and the hook took those replies for the peer's first direct packet. JS avoids the case differently (its relay does not advertise LAN addresses, `shareLocalAddress: false`).

Beyond F1-F13 the fork also has: `SecretStream` keepalive every 5 s, `seq` wrap-around at 2^32, ACK on stream END;
`connect` without holders returns `PeerNotFound` (`NoRelayNodes` if all holders refused); holders are tried two at a time
as `findAndConnect` does in JS; libudx pacing as a token bucket after the reference `udx.c`;
large files split into directories with unchanged public paths.

These changes are not sent upstream.

## Comparing with upstream

```sh
git clone --branch peeroxide-v1.7.3 https://github.com/Rightbracket/peeroxide /tmp/peeroxide-upstream
diff -r --exclude=.git --exclude=.github /tmp/peeroxide-upstream fork/peeroxide
```

shows every change (except this file).

## License

The fork is `MIT OR Apache-2.0` (`LICENSE-MIT`, `LICENSE-APACHE` in this directory); our crates are `AGPL-3.0-only`.
