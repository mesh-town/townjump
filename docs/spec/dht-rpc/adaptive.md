# dht-rpc adaptive mode: how a node turns itself persistent

Source: `dht-rpc/index.js` (6.27.0): constructor (`adaptive`, `_forcePersistent`), `_updateTick`, `_updateNetworkState`, `_onwakeup`, `_onrequest` (`PING_NAT`), `_onresponse`, `remoteAddress`. Implemented in the fork (`peeroxide-dht`): `rpc/adaptive.rs` (countdown), `rpc/nat_check.rs` (PING_NAT round), `rpc/node_adaptive.rs` (wiring), `rpc/handle_mode.rs` (`DhtHandle::mode`, `watch_mode`, `set_adaptive`).

## When the mode is on

`dht.adaptive = typeof opts.ephemeral !== 'boolean' && opts.adaptive !== false`. A node with an explicit `ephemeral` never changes mode. A node with a non-empty bootstrap list starts ephemeral; an empty list means "own bootstrapper", which is persistent from the start. In the fork: `adaptive && ephemeral.is_none() && !bootstrap.is_empty()` (`DhtConfig::adaptive`, default true).

## Countdown

Every 5 s tick, after the node is bootstrapped, `_updateTick` decrements a counter while the node is ephemeral. The initial value is `STABLE_TICKS = 240` (20 min); fork: `DhtConfig::stable_ticks`. At zero the node runs `_updateNetworkState`, which sets the counter to `3 * STABLE_TICKS` and remembers the current NAT host. Until the host changes (`_nat.host`, compared with `===`, so `null === null` counts as "unchanged") the node does not check again: a failed check is not retried on the same address. A persistent node does not count.

## `_updateNetworkState`

1. Do nothing if the node is persistent or a check is already running.
2. Take a snapshot of the NAT window address (`host:port` agreed by the last 16 replies). No address: stop.
3. Not firewalled (`firewalled: false` in the config): promote at once with the snapshot.
4. Firewalled: ask nodes `PING_NAT` (see below); the targets are the 5 routing-table nodes seen most recently, followed by all bootstrap nodes when the table gives fewer than 5.
5. Decision: replies that arrived on the server socket, at least 1 (3 when 5 or more nodes were asked), and the NAT window must still report the same host and, as port, the server socket's port. Otherwise the node stays ephemeral.

## `PING_NAT`

The request value is the sender's server-socket port (u16, little-endian). The receiver answers to `request.from.host : value-port` (a deferred reply, from the server socket), not to the address the request came from. The sender counts the reply only if it arrived at its server socket (`hosts.has(res.from.host)` in `_onresponse`); a reply to the client socket proves nothing. A zero or short value gets no reply. Replies of this round do not feed the NAT sampler.

## Promotion

When the check passes, the node sets `firewalled = false`, then promotes if it is still ephemeral, adaptive, the NAT window still holds the snapshot address and the reachable address has the same host and a non-zero port: `ephemeral = false`, the routing-table id becomes `peer_id(host, port)` (the table is rebuilt and refreshed), and the node starts answering routing requests.

## Wake-up

`_onwakeup` (the tick was paused for more than 15 s, or `resume`): the counter is set to `3 * STABLE_TICKS`, the last host is forgotten, and an adaptive persistent node goes back to ephemeral. The table id is kept. The JS comment about resetting `firewalled` there is commented out (TODO); the node stays `firewalled = false`.

## Extensions in the fork (not in JS)

- `wake_up` demotion also restores `firewalled` from the config and closes the published open window, so a machine that changed networks while asleep does not announce `FIREWALL_OPEN` and its next promotion again goes through `PING_NAT`.
- `DhtHandle::set_adaptive(false)` on a node that became persistent through adaptive demotes it to ephemeral (the id is kept, `firewalled` untouched); `set_adaptive(true)` on an ephemeral node restarts the countdown; on a node persistent by config it does nothing. JS reads the flag only in the constructor.
- `remote_state` of a promoted node reports the window address with `FIREWALL_OPEN` while the window port equals the server-socket port seen at promotion (`NodeNat::open`). JS `remoteAddress` returns `{host, port}` for any node that is not firewalled, when the window port equals the current server-socket port. The fork differs in one place: a node with `firewalled: false` in the config (never promoted) still reports no address.
- Observation: `DhtMode { ephemeral, adaptive, firewalled, address }` through `DhtHandle::mode()` and a `tokio::sync::watch` (`watch_mode()`); JS has only the `persistent` event.

## Not ported

`quickFirewall` in `_bootstrap`, `nat-update` events and `_nonePersistentSamples`, the `-2` offset in `_onwakeup`, a CLI flag (the factory reads `TOWNJUMP_DHT_ADAPTIVE=0`).
