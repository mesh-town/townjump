# Wire deltas: what changed between the old JS versions and the reference

Audience: contributors who modify peeroxide and capture test vectors. This covers what changes the bytes on the wire, or the moment they are sent, between the older JS versions that upstream peeroxide follows and the reference of this port (JS holesail 2.4.4). Reference versions: holesail 2.4.4, hyperdht 6.34.0, dht-rpc 6.27.0, udx-native 1.21.3. D11 onwards and the last sections go beyond the version changes proper.

Version pairs (old → reference): libudx `25b063b` → `ae8bff7` (udx-native 1.19.2 → 1.21.3), hyperdht 6.30.0 → 6.34.0, compact-encoding 2.19.2 → 3.5.2, protomux 3.10.1 → 3.12.1, blind-relay 1.4.0 → 1.6.1, dht-rpc 6.26.4 → 6.27.0, hypercore-crypto 3.6.1 → 3.7.0, @hyperswarm/secret-stream 6.9.1 → 6.9.2.

## How to read

- The source is `package/file:symbol`, without line numbers. The place in peeroxide is `crate/file:symbol` (path relative to `fork/peeroxide/`: `libudx/src/native/stream.rs`, `peeroxide-dht/src/…`). The Rust locations below refer to upstream 1.7.3; in the fork `hyperdht.rs`, `rpc.rs`, `io.rs`, `libudx/src/native/stream.rs` are split into directories (same behavior), so look the symbol up by name: `process_incoming`, RTO (`rto_sleep`) — `native/stream/incoming.rs`; `on_sack` — `stream/sack.rs`; `send_ack` — `stream/wire.rs`; `MAX_RTO_TIMEOUTS`, the RTO handler — `stream/consts.rs`, `stream/timers.rs`; `update_rtt`, `detect_losses` — `stream/loss.rs`; `mtu_probeify_packet` — `stream/mtu.rs`; sending DESTROY — `stream/destroy.rs`; `is_addr_private`, `should_direct_connect` — `hyperdht/connect_finish.rs`; connection via relay — `hyperdht/connect_relay.rs`; the hyperdht request loop — `hyperdht/node.rs`.
- Evidence: byte layouts and timings were captured from JS runs (scripts below); statements about code paths come from reading the JS sources and were not verified at runtime.
- Captures used `udx-native` 1.21.3 (the reference, installed by `interop/setup-js.sh`) and 1.19.2 (the older version), Node 24.4.0. The scripts are in [testvectors/deltas-js/](../../testvectors/deltas-js/): `plugin.js` (PLUGIN codec), `udxcap.js` (empty MESSAGE and HEARTBEAT from keepalive), `udxzwp.js` (zero-window probes), `udxrto.js` (silent peer). The peer in them is a plain Node UDP socket, so packet bytes are fully visible. Run: `UDXPATH=<path to udx-native> NODE_PATH=<node_modules with b4a and compact-encoding> node <script>`.
- The UDX header is 20 bytes, all little-endian: `[0]=0xFF` magic, `[1]=1` version, `[2]` type flags (DATA 0x01, END 0x02, SACK 0x04, MESSAGE 0x08, DESTROY 0x10, HEARTBEAT 0x20), `[3]` data_offset, `[4..8)` remote_id, `[8..12)` receive window, `[12..16)` seq, `[16..20)` ack. A SACK block is a pair of u32 `(start, end)`, `end` exclusive.

## Summary

| № | Change | Visible on the wire | Handling in the fork |
|---|---|---|---|
| D1 | PLUGIN command = 10 and `NS_PLUGIN` (hyperdht 6.34) | not for our path: nobody replies | constant PLUGIN = 10, `NS_PLUGIN` and the codec; no logic |
| D2 | Empty UDX MESSAGE packet after the relay → direct switch; the relay closes on confirmation | yes, 20 bytes | `change_remote` and the switch (FORK.md F11); see D2 and D9 |
| D3 | HEARTBEAT flag 0x20 | yes, a new packet type | the reply to HEARTBEAT is an ACK, with an exit before window/ack/SACK handling; there is no one-second timer, keepalive lives at the SecretStream level (5 s, like hyperdht `connectionKeepAlive`) |
| D4 | The zero-window probe is a HEARTBEAT, not DATA | yes | zero-window probe immediately, then after rto, then rto×2 |
| D5 | UDX-level keepalive (off by default) | only if enabled | not enabled |
| D6 | Strict SACK block validation | only on malformed packets | predicate `remote_acked ≤ start < end ≤ seq` (mod 2^32) |
| D7 | RTO, TLP, RTT | timings | the 7th RTO firing → ETIMEDOUT (~13 s), RTT sample 0 → 1; TLP is not implemented |
| D8 | ACK for every DATA/END synchronously | number and timing of ACKs | ACK on the acknowledgement of our END, as in JS |
| D9 | `change_remote`, MTU probes | yes | `change_remote` in F11; MTU probes: see D9 |
| D10 | DESTROY, libudx relay | DESTROY timing; relay — no | mostly no |
| D11 | hyperdht: server LAN pre-holepunch, LAN-label race with the holepunch (beyond the list) | source address of the handshake reply | the handshake reply source is as in JS |
| D12 | compact-encoding: strict `c.buffer`, integer bounds, `Out of bounds` (beyond the list) | receive: yes | strict decoder |
| D13 | protomux: length clamp within a batch (beyond the list) | reply frame | strict parsing: an item that overflows the frame is an error |
| D14 | blind-relay: a one-sided unpair tears down both streams (beyond the list) | on the relay server | relevant if the Rust server is a relay |
| D15 | No wire changes: dht-rpc, hypercore-crypto, secret-stream, protomux channel id | no | nothing |

## D1. PLUGIN = 10 and NS_PLUGIN

**Was** (hyperdht 6.30.0): commands 0..9, no code for 10. **Now** (6.31.0, commit `cb22d78` "Plugin system"): `hyperdht/lib/constants.js:COMMANDS.PLUGIN = 10`; `NS.PLUGIN` is the 6th element of `crypto.namespace('hyperswarm/dht', [ANNOUNCE, UNANNOUNCE, MUTABLE_PUT, PEER_HANDSHAKE, PEER_HOLEPUNCH, PLUGIN])`.

- `NS_PLUGIN` (32 bytes, id 10) = `6b8a2a8623f28c6c02a1e22abe515ce31cafd9af61d4b69970639aa7eb96a8d5`. It is not used in any signature: the constant is introduced but not applied (neither `announce` nor `handshake` takes it).
- The request value `pluginRequest` (`hyperdht/lib/messages.js:pluginRequest`): `c.string plugin` · `c.uint version` · `c.uint command` · `c.uint flags` · if `flags & 1`, `c.buffer value`. The flag is set by `if (m.value)`. Vectors (plugin `"x"`, version 1, command 2): `value = null` → `0178010200`; `value = empty Buffer` → `017801020100` (an empty Buffer is truthy, flag 1, length 0); `value = aabb` → `017801020102aabb`. Decoding gives `null` / empty Buffer / `aabb` respectively.
- Receive (`hyperdht/index.js:_onrequest`, branch `COMMANDS.PLUGIN` → `hyperdht/lib/persistent.js:onplugin`): works only on a persistent node with an id; silently (no reply, no error) returns if `req.value` is empty, decoding gave `null`, the plugin is not registered, or `version` differs from the registered one. Only `plugin.onrequest` sends a reply. Holesail has no plugins, so there will never be a reply.
- Behavior for command 10: 6.30.0 has no branch, `_onrequest` returns `false`, and `dht-rpc/index.js:_onrequest` replies with the error `UNKNOWN_COMMAND` (code 1). 6.34.0 is silent, but only on a persistent node with an `id`; on a node without an `id`, e.g. a localhost testnet, both versions reply `UNKNOWN_COMMAND` (`testvectors/deltas/plugin.json`). On a persistent node with an `id`, 6.34.0 is silent for an empty `value` and for an unregistered plugin, and replies for a registered one; 6.30.0 replies `UNKNOWN_COMMAND` (no branch). For a Rust node this means: command 10 must be silent. PLUGIN is released by an explicit `release`, and other unknown commands get `UNKNOWN_COMMAND` ("Beyond the list", item 3).

**Where it lives.** `peeroxide-dht/src/hyperdht_messages.rs` (next to `IMMUTABLE_GET = 9`: `PLUGIN: u64 = 10`, the `PluginRequest` codec); `peeroxide-dht/src/crypto.rs` (next to `NS_ANNOUNCE`: `NS_PLUGIN = namespace("hyperswarm/dht", &[10])[0]`); the request loop is `hyperdht/node.rs` (`match req.command`), where PLUGIN and `_ =>` are explicit branches ("Beyond the list", item 3).

**Vectors.** `NS_PLUGIN`; the three `pluginRequest` encodings above; an empty `req.value` on input of command 10 → no reply.

## D2. Empty UDX packet on the switch from relay to direct connection

**Was** (hyperdht 6.30.0 `lib/connect.js:onsocket`): only `changeRemote(...)` and `remoteChanging.catch(safetyCatch)`; the relay socket was not closed and there was no empty packet. Intermediate (6.32, `a9371f4`): closing the relay on the `remote-changed` event. **Now** (6.34, commit `120364a`, superseded the intermediate): `hyperdht/lib/relay-connection.js:confirmDirectUpgrade(c, rawStream, remoteChanging)`. It first waits for the switch to finish (`remoteChanging` is a promise if the switch is deferred because of unacknowledged packets), then:

1. attaches `data` and `message` listeners to the raw UDX stream;
2. `rawStream.trySend(b4a.alloc(0))` — an **empty UDX MESSAGE**: 20 bytes, type 0x08, no data. Bytes (stream id 5 connected to `remote_id` 7, receive window 4 MiB): `ff01 08 00 | 07000000 | 00004000 | 00000000 | 00000000`. `seq` is not spent (MESSAGE is outside the DATA numbering), `ack` is the current one;
3. the first packet to arrive on the direct path closes the relay (`closeRelayConnection` → `socket.end()`). "Arrived on the direct path" means a packet on which the stream's `firewall` with the "socket is a relay" flag did not fire (`connect.js`/`server.js:firewall` set `validUpgrade = false` on a packet from the relay and `true` on a packet from another socket).

Why: without a packet coming back, the peer whose switch had not happened yet did not see the direct path, while closing the relay right after `remote-changed` left one side unable to acknowledge packets that had gone through the relay.

What the receiver sees: a MESSAGE packet triggers `on_recv`/the `message` event even at zero length (`libudx/src/udx.c:process_packet`, branch `UDX_HEADER_MESSAGE`). Type 0x08 existed in the old libudx too, so an old receiver will accept the packet.

**In the fork.** Upstream peeroxide 1.7.3 has `libudx/src/native/header.rs:FLAG_MESSAGE`, but `process_incoming` does not parse the MESSAGE type and libudx has no `change_remote`. Upstream has no relay → direct path either: the relay connection function (`hyperdht/connect_relay.rs` in the fork; it calls `BlindRelayClient::open` and `data_stream.connect(...)`) stays on the relay until the connection ends. For wire compatibility the delta alone is not mandatory: a node that never switches to direct still interoperates. The fork implements the switch (FORK.md F11): a method to change the remote address in `native/stream/` (D9), sending the empty MESSAGE, parsing an incoming MESSAGE, and the rule for closing the relay on an acknowledged packet.

**Vectors.** The empty MESSAGE (bytes above); the "relay → direct" scenario: the packet sequence after `change_remote` (needs two JS nodes; see "Open points").

## D3. HEARTBEAT flag 0x20

**Was** (libudx `25b063b`, `include/udx.h`): five flags (0x01..0x10), the sender did not send "header-only" packets. **Now** (`60cf0cf` "Keepalive", `d3e199e`): `UDX_HEADER_HEARTBEAT 0b100000`.

- The packet (`libudx/src/udx.c:send_probe`): exactly 20 bytes, `[2]=0x20` with no other bits, `[3]=0`, `[8..12)` receive window, `[12..16)` seq = the next unsent one (seq is not consumed), `[16..20)` ack = the current one. It is not put on the send queue, not retransmitted, and ignores cwnd and pacing. Bytes: `ff01 20 00 | 07000000 | 00004000 | 00000000 | 00000000`.
- Receive (`udx.c:process_packet`): after checking magic, version and looking up the stream by `remote_id`, if the stream is a relay, the packet is forwarded as is; otherwise `if (type & UDX_HEADER_HEARTBEAT) { send_ack(stream); return 1; }`. All of this happens before parsing data_offset, DATA/END, SACK, window and ack: the probe fields (window, seq, ack) are ignored entirely. The reply is a plain ACK (D8).
- The old receiver (`25b063b`): does not check bit 0x20, the packet goes down the common path (updates the window from the probe fields, does not advance ack) and **sends no reply**, because an ACK is queued only for DATA/END. So only the new receiver gives a live reply to HEARTBEAT.

**Upstream peeroxide 1.7.3** (`libudx/src/native/stream.rs:process_incoming`): on input of 0x20 it sends an ACK via `send_ack`; but before that it processes the packet's `ack` and window (`send_rwnd = header.recv_window`, `on_cumulative_ack`), while the reference ignores the probe fields. Its probe comes from the `keepalive.tick()` branch every `HEARTBEAT_INTERVAL_MS = 1000` ms when idle (`connected && outgoing.is_empty() && relay_target.is_none()`), with header `type=0x20, seq=0, ack=next_remote_seq, window=DEFAULT_RWND`. In the reference the stream does not send 0x20 when idle by default (D5), and the probe `seq` is the current `stream->seq`, not 0.

**In the fork.** `libudx/src/native/stream/`: (a) on input of 0x20 — reply with an ACK and exit before handling ack/window; (b) the probe carries `seq = next send seq`, not 0; (c) there is no 1 s timer, as in the reference (see D5). The test in `header.rs` (`FLAG_HEARTBEAT`) stays.

**Vectors.** A HEARTBEAT packet (bytes above) from JS; JS's reply ACK to a fed 0x20 (feed a raw packet to a JS stream through a UDP socket and capture the reply; script — modeled on `udxzwp.js`).

## D4. Zero-window probe

**Was** (`libudx/src/udx.c:udx_zwp_timeout`): timer `zwp_timer` with period `rto`; when it fired the flag `WANT_ZWP` was set, and the send loop sent a **real new DATA packet** from the write queue (seq consumed, the packet in inflight with RTO and retransmit). **Now** (`d3e199e`, `8f41cc8`): the probe is a HEARTBEAT (D3), no data goes out, seq is not spent. Start: on the first write to an idle stream with `send_rwnd == 0` the probe goes out **immediately**; then after `rto`, then every `rto × 2` (the current `rto`), with no limit on the count. `send_rwnd_in_packets` returns at least 1 for any window above 0; retransmits are not limited by the window.

Capture (a raw peer sends an ACK with window 0, then JS writes 5 bytes; script `udxzwp.js`): the reference sends 0x20 at 103 ms, 1105 ms, 3105 ms, 5107 ms (intervals 1 s, then 2 s, 2 s); the old version sent DATA (25 bytes, body `hello`) at 102 ms and sent nothing over the next 7 s, including no retransmit (the missing retransmit is not explained, see "Open points"; per the source the old probe is DATA).

**In the fork.** Upstream peeroxide 1.7.3 has no zero-window probes (`send_rwnd` is used only in `can_send`), so with window 0 it stops sending without a probe. The fork adds a ZWP timer that sends a HEARTBEAT by the rules above.

**Vectors.** Probe timings (numbers above, tolerance ±50 ms); 0x20 packets at window 0.

## D5. Keepalive

UDX level (`udx.c:udx_stream_set_keepalive`, `60cf0cf`): when `ms` is set, on an idle stream (`remote_acked == seq`) it sends a HEARTBEAT every `ms`, there is no reply deadline — a silent peer does not kill the stream. **Off by default**: `setKeepalive` is not called anywhere in hyperdht, udx-native, dht-rpc or secret-stream (`rg`). A working `setKeepalive(300)` gives 0x20 every ~300 ms (`udxcap.js`).

Separately, the secret-stream keepalive (`connectionKeepAlive: 5000` in hyperdht → `@hyperswarm/secret-stream:sendKeepAlive`): an **empty message** via `write(alloc(0))`, on UDX an ordinary DATA packet with an encrypted body; this is the JS level, not 0x20. Secret-stream 6.9.1 → 6.9.2 (`fcfa24b`): an empty message is dropped on receive if `filterZeroByteMessages` is set or `keepAlive > 0` (previously `keepAlive !== 0`); the bytes did not change.

**Consequence for Rust:** a HEARTBEAT once a second on an idle stream (D3) is behavior the reference does not have by default; an old node will not answer it, and a new one will answer with an ACK. Rust has no HEARTBEAT timer, as in JS.

## D6. Strict SACK validation

**Was** (`25b063b:process_packet`): SACK blocks were not validated; `for (p = start; p != end; p++) ack_packet(...)` — for `end < start` the loop runs ~2³² steps. **Now** (`70b1438`, `libudx/src/udx.c:udx_sack_is_valid`): a block is accepted only if `remote_acked <= start < end <= seq` (all comparisons modulo 2³² via `seq_diff`). The check runs after the `ack <= seq` check and after `remote_acked` is updated with the new `ack`. An invalid block: `dropped_sacks++`, the block is skipped, the rest of the packet is processed. The intermediate `0420f62` dropped the whole packet — it is not in the reference.

The number of blocks in an ACK: at most 50 (`UDX_MAX_SACKS`; the old version, by the loop arithmetic, up to 51), packet length `20 + 8·n`, `data_offset = 0`, type `SACK` when `n > 0`. The number of blocks on receive is unlimited: `nsack_blocks = header_len / 8`, `header_len = (0 < data_offset < payload_len) ? data_offset : payload_len`. Visible on the wire only on malformed packets; valid ACKs are identical.

**Upstream peeroxide 1.7.3** (`stream.rs:on_sack`): the loop `for seq in range.start..range.end` without a predicate; when `end` is much larger than `start` it is a long loop (potentially expensive on a crafted packet from a remote peer). SACK is received under the condition `FLAG_SACK && data_offset > 0` (see "Beyond the list" below: this differs from the reference).

**In the fork.** `on_sack` (and the calling branch in `process_incoming`) applies the predicate `remote_acked <= start < end <= seq` (modulo 2³² arithmetic), skips an invalid block and processes the rest, after `remote_acked` is updated.

**Vectors.** A set of ACK packets with blocks: valid `(remote_acked, seq)`; `end == seq`; `start < remote_acked`; `end > seq`; `start >= end`; a mix of valid and invalid. Observable: which packets are marked received (retransmits after the ACK); the JS side does not output `dropped_sacks`, so on the wire this shows up as retransmits, and the vector is not captured.

## D7. RTO, TLP, RTT

- **Constants (unchanged):** `UDX_MAX_RTO_TIMEOUTS 6`, `UDX_RTO_MAX_MS 30000`, initial `rto = 1000`, `rto = max(srtt + 4·rttvar, 1000)`.
- **RTO counter** (`80aabcf`). Was: a counter on the packet; the `>= 6` check only for the queue head that passed the condition `seq == remote_acked || remaining < 0`, packets marked lost by RACK were skipped without counting. Now: a stream counter `rto_count++` on every firing, `if (rto_count > 6) close(ETIMEDOUT)` (on the 7th), reset only by `ack` advancing (SACK does not reset it). Backoff is the same in both: after firing the timer is set to `rto × 2`, `rto` itself does not grow.
  Capture with a silent peer (`udxrto.js`): both versions behave the same — DATA at 3 ms and retransmits at ~1, 3, 5, 7, 9, 11 s, `ETIMEDOUT` at ~13 s. So with total silence the change is not visible; it shows only when the queue head is already marked lost (loss + SACK progress).
- **TLP** (`schedule_loss_probe`): was `pto = srtt×2 (+2 ms with one packet in flight)` when `srtt != 0`, otherwise 1000; now the same, but "one packet" means `inflight_queue.len + retransmit_queue.len == 1`, the ceiling is the time remaining to RTO, and it is not armed when `seq == remote_acked`, `ca_state != OPEN` or a SACK is present.
- **RTT** (`12e5c39`): a sample of 0 ms becomes 1 (`if (rtt == 0) rtt = 1`), because `srtt == 0` was read as "no sample" and gave a TLP of 1000 ms. On fast links (localhost, LAN) the probe packet goes out after `1·2 + 2` ms, not 1000.
- **ssthresh** (`01d6630`): the field and leaving RECOVERY by it were removed; on RTO `cwnd = inflight_queue.len + 1`; `bbr_inflight ≥ 1`. Only the speed after losses; the packet shape is the same.
- Single timer (`8f41cc8`): RTO, TLP, RACK, ZWP, keepalive share one `stream->timer`; the order of events is the same.

**Upstream peeroxide 1.7.3** (`stream.rs`): `MAX_RTO_TIMEOUTS = 6` and the check `rto_timeouts >= 6` — failure on the **6th** firing; after each firing `guard.rto` itself is doubled (up to 30 s) and the timer is set to the new value. Rust intervals: 1, 2, 4, 8, 16, 30 s (firings at 1, 3, 7, 15, 31, 61 s) — a drop at ~61 s versus 13 s in JS. The RTT update (`update_rtt`) does not change the `srtt == 0` branch on 0 ms: `if self.srtt == 0` takes the first sample, and a sample of 0 leaves `srtt == 0`. TLP is absent in Rust (fast retransmission on a SACK gap `detect_losses`, a simplified RACK, a comment in the code).

**In the fork.** In `stream/`, the `rto_sleep` handler counts firings with the threshold `> 6` and waits `rto × 2` after a firing without growing `rto`; `update_rtt` turns a sample of 0 into 1; TLP is not implemented (the fast path affects speed, not compatibility).

**Vectors.** The moments of retransmits and `ETIMEDOUT` with a silent peer (numbers above, tolerance ±50 ms); the same moments with a peer sending SACK without advancing `ack` (not captured); the RTT sample on localhost — the TLP interval.

## D8. ACK for every DATA/END

**Was** (`25b063b`): on DATA/END the flag "state needed" is raised, and the ACK packet (with SACK) is built later in the send loop, one per iteration. **Now:** `process_packet` — `if (type & UDX_HEADER_DATA_OR_END) send_ack(stream)` synchronously for every received DATA/END, including duplicates; on HEARTBEAT and after the END is acknowledged too. Before `connect` the ACK is deferred by the `ack_needed` flag and goes out in `udx_stream_connect`. There is no delayed-ACK timer. The number of ACKs grows (one per packet), the timing is immediate. The commit that made it synchronous is not identified (probably `bbb5901`, the switch from poll to `uv_udp`).

**Rust:** `process_incoming` sends `send_ack` at the end of the DATA/END branch for every packet, so it matches; there is no delay. On the acknowledgement of our END, both JS versions (1.19.2 and 1.21.3) send exactly one ACK (`seq` = next number, `ack` = received) if the peer has not finished; a repeated ACK gets silence; if the peer has finished, silence and close. Rust does the same.

**Vectors.** N consecutive DATA packets at a JS receiver → N ACKs (`ack`, SACK blocks at gaps) with timestamps.

## D9. `change_remote` and MTU probes

- **`udx_stream_change_remote`** (`be51b3a`): the switch point `seq_of_change = stream->seq + (a DATA packet is being built ? 1 : 0)` instead of a bare `seq`; `defer_change = seq_of_change != remote_acked`; the MTU is reset to the base one (`reset_mtu_state_machine`, `mtu_max = UDX_MTU_MAX`); an empty packet builder takes the new MTU immediately; an MTU probe is not scheduled during the switch. `udx_write_header` receives `remote_id` as an explicit parameter. What goes out right after the switch and the behavior of `udx_stream_connect` on a connected stream are not examined.
- **MTU probe** (`mtu_probeify_packet`): was — `pkt->header[3] != 0` cut it off, padded up to `wanted_size`, limit 255; now — only packets with the DATA flag (`header[2] & DATA`), `padding_size = wanted_size − (size + ip_udp_overhead)`, `padding_size ∈ [0, 255]`. Format: a plain DATA, `data_offset = padding_size`, zeros between the header and the data; receive: `data_offset > buf_len` — the packet is dropped, otherwise the data is after `data_offset`.
- Upstream peeroxide 1.7.3 has no `change_remote` (D2; the fork adds it, FORK.md F11). MTU probes exist (`stream.rs:mtu_probeify_packet`, in the fork `stream/mtu.rs`, 32-byte step, `MTU_MAX_PROBES = 3`): a probe is attached only to DATA and `data_offset` does not exceed 255.

**Vectors.** Packet headers after the remote address change (`remote_id`, address, size) — needs two JS streams; MTU probe: DATA with `data_offset != 0`, the datagram length equals `wanted_size`.

## D10. DESTROY and the libudx relay

- **DESTROY:** the new `udx_stream_destroy` for a connected stream sends a 20-byte 0x10 packet immediately (`seq` before the increment, then `seq++`); the old one set `WANT_DESTROY` and sent on the next pass of the loop (the old code's send time is not verified). Receive: `seq_compare(stream->ack, seq) <= 0` → `close(ECONNRESET)`, unchanged. Rust sends DESTROY (`stream.rs`, `type_flags: FLAG_DESTROY`); it receives it as `End` and closes.
- **libudx relay** (`fa576bd`, `759bf76`): relative to the old version there is no wire difference (the `bbb5901` regression is fixed); the packet is forwarded as is through the target stream's socket; DESTROY is not sent on a relayed stream.
- **MESSAGE:** `udx_stream_send` sends 20 bytes + body, zero length is allowed; receive calls `on_recv` even when `buf_len == 0`.

## D11. hyperdht: LAN pre-holepunch and the handshake reply (beyond the list)

- **Handshake reply from the puncher socket** (`69bfcf0`, hyperdht 6.33.0, `lib/server.js:_addHandshake`, the `_reply` path: `replyFromPuncher`). Was: the server created a puncher and replied from the server socket. Now: if the client is on the **same network** (the same external address `clientAddress.host === serverAddress.host`, a direct connection not through a relay) and the client has private addresses (`isPrivate`), the puncher is created for the LAN pre-holepunch, but `replyFromPuncher` stays `false`, and in the handshake the server still announces the **ordinary** server socket; in other cases `replyFromPuncher = true` and the puncher socket is announced (`return { socket: h.replyFromPuncher ? h.puncher.socket : null, noise: h.reply }`). On the wire, what changes is the source UDP port of the handshake reply; the `holepunch` part is the same.
- **LAN-shortcut race with the holepunch** (`400f2cb`, `lib/connect.js`): was — `await c.dht.ping(LAN address)`; a failed ping aborted the connection (`HOLEPUNCH_ABORTED`). Now — the ping runs in parallel with the ordinary holepunch; a successful ping calls `onsocket` (whoever arrives first takes `rawStream`), a failure does not interrupt; if the remote node cannot be holepunched, the LAN is the only path. On the wire: parallel probes to private addresses at the same time as the holepunch instead of a strictly preceding ping.
- **Relay keepalive and the tail** (`120364a`): `relayKeepAlive: 5000`, the server keeps the relay socket alive; the relay connection's keepalive is an ordinary HYPERDHT channel, no separate bytes were introduced.
- Other changes in 6.30 → 6.34 (`7869f2f`, `ac4b1da`, `4b91712`, `ac6eaa5`, `2e7d760`, `1c5080a`: handshake cleanup, statistics, invalid ports in the firewall) — local behavior, they do not change the wire.

**Rust:** `hyperdht/connect_finish.rs` has `is_addr_private`, `should_direct_connect(relayed, firewall, remote_holepunchable)`, and `holepuncher.rs` has `match_address`. The socket of the handshake reply is decided by `handle_server_handshake` (`hyperdht/server_run.rs`; the new server — `server/handshake.rs`) and `handle_peer_handshake` (`hyperdht/node_handshake.rs`); what JS does is shown in the two blocks below.

**JS facts about which socket the server replies from to a handshake** (probe `testvectors/probes-js/fork-d11-probe-js.mjs`, output `fork-d11-probe-js.out.txt`, hyperdht 6.34.0, a testnet of 8 nodes on 127.0.0.1; "fw" is a node with `firewalled = true`, forced by `_checkIfFirewalled = async () => true`; the client is fw too, otherwise it is open and no puncher is created). Server sockets: `server` (port `local_port()`), `client` (outgoing, on which an fw node receives FROM_RELAY), `pool` (the puncher socket).
- open: direct — received on `server`, the reply from `server` to the client; relay — from `server` to `req.from` (holder H1); second (FROM_SECOND_RELAY: client → H1 with `relayAddress`=R2, H1 → R2, R2 → server) — from `server` to `relayAddress` (H1), not to `req.from` (R2).
- fw without LAN: direct — received on `server` (not on `client`!), the reply from `server`; relay — received on `client`, the reply from **`pool`** to `req.from`; second — received on `client`, the reply from `pool` to `relayAddress`.
- fw with a LAN shortcut (the client on the same host with a private address, `shareLocalAddress`): direct — as above; relay/second — received on `client`, the reply **from `client`** (the puncher is created, but `replyFromPuncher = false`).

The client sees in the reply `serverAddress` = the FROM_SERVER source address as seen by the holder (fw without LAN — the puncher port; open — the `server` port): that is the NAT mapping for the holepunch.

**The mechanism in JS and where Rust differs.** `hyperdht/lib/server.js:_onpeerhandshake` returns `{ socket: h.replyFromPuncher ? h.puncher.socket : null, noise }`; `null` means "the receiving socket" (`dht-rpc/lib/io.js`: `opts.socket || this.socket`). `replyFromPuncher = true` only in the last branch of `_addHandshake` (the request is not direct, the client is not open, the server has no `remoteAddress()`, no LAN shortcut); for direct and an open client `hs.onsocket(direct ? socket : dht.socket, …)`. `router.js:onpeerhandshake`: FROM_SECOND_RELAY without `relayAddress` — silence, otherwise `req.relay(…, relayAddress)`. The fork reproduces this (`handshake_route.rs`, `Io::relay(.., via)`, `relay_target`); the only difference is on the LAN side. A client behind the server's own public address (`server/plan.rs::same_host`, kept for the log only) gets `Plan::Holepunch` like any firewalled client: the puncher is created and the server replies from the puncher socket. The client side, when `same_host` and `local_connection` hold, checks the private addresses from the server's advertisement on the holepunch branch, in parallel with the ordinary holepunch rounds: one address at a time, up to 2 s each, with a 1-byte punch probe (reflected by the server's passive puncher) plus a DHT ping to the node socket; the first reply counts. The first success wins and the other path is cancelled; if both fail, the connection ends with `HolepunchFailed` instead of a connection on a dead stream. Unlike JS, `matchAddress` is not used, and the client announces its local addresses only with `local_connection` (JS `c.lan`). A firewalled JS client does not announce its reflexive address (`dht-rpc` `remoteAddress()` is null), so it fails (b) and keeps the holepunch path; a non-firewalled JS client announces `FIREWALL_OPEN` and gets `DialClient` before the gate. A Rust server is never `OPEN` on the public DHT (a Rust node never clears `firewalled`: it sends no outgoing `PING_NAT`), so a client from another public IP always gets the holepunch plan.

**Server announce in JS** (`hyperdht/lib/announcer.js`, 6.34.0). After a successful ANNOUNCE (`_commit`, :264-265) `{ relayAddress: msg.from, peerAddress: msg.to }` goes into `relays` (`msg.from` is the relay node, `msg.to` is our address as the relay sees it), and `msg.from` goes into `relayAddresses` (at most 3); with `!dht.firewalled` our own `remoteAddress()` comes first (:174-177). The background (:111-125): a loop of up to 100 steps with a 3 s pause (~5 min), pinging relays; fewer than `min(relays, MIN_ACTIVE = 3)` active — `refresh()`; then a new announce. `stop()` withdraws the announce from all known relays. The client picks the server's relay by matching `relayAddress` (`pickServerRelay`, `connect.js:~850`, otherwise `relays[0]`) and sends PEER_HOLEPUNCH with `peerAddress = serverRelay.peerAddress` to `serverRelay.relayAddress` (`updateHolepunch`, `connect.js:~550`) — so `peer_address` in the server's reply must be `msg.to`, not the client's address.

## D12. compact-encoding 2.19.2 → 3.5.2 (beyond the list)

These change receive or encoding: `b302a4c` (3.0.0), `5636fb6` (3.2.0), `2af5451` (3.3.0), `d64fab1` (3.3.1), `c5f2047` (3.5.2). The rest are new codecs, fast paths, types; the bytes are the same (fuzz of 20000 strings and 10000 floating-point numbers: no differences in encoding).

| What | Was → now | Visible on the wire |
|---|---|---|
| `c.buffer`/`c.binary` from `null` (`compact-encoding/index.js:buffer`) | writes byte `00` → throws `TypeError` | encoding: no (the holesail path does not pass `null` to `c.buffer`) |
| `c.buffer` of length 0 on receive | `null` → empty `Buffer` | yes: `hyperdht/lib/router.js:onpeerhandshake` is called for an empty `noise` (before, `null` — not called) |
| `c.raw.buffer` empty | `00` (1 byte) → empty (0 bytes) | yes, but not found on the holesail path |
| `c.uint` / `int` / `uint56` / `uint64` beyond ±2⁵³ | receive and write without a check → throws (`uint must be between 0 and 9007199254740991`; for `int` — ±2⁵² ) | receive: the `0xff` prefix with a value ≥ 2⁵³ is now rejected |
| `c.frame`, `c.arraybuffer`: length past the end | read past the end → `Out of bounds` | receive: yes |

`c.uint`: 0 → `00`, `0xfc` → `fc`, `0xfd` → `fdfd00`, `0xffff` → `fdffff`, `0x10000` → `fe00000100`, `0xffffffff` → `feffffffff`, 2³² → `ff0000000001000000`, 2⁵³−1 → `ffffffffffffff1f00` — the bytes are identical in both versions; 2⁵³ (`ff0000000000002000`) — the old one writes it, the new one throws; decoding `ff0000000000002000` — the old one gives 9007199254740992, the new one throws.

**Upstream peeroxide 1.7.3** (`peeroxide-dht/src/compact_encoding.rs`): `decode_buffer` for length 0 returns `None` (2.x behavior, `if len == 0 { return Ok(None) }`); `decode_uint`/`decode_uint64` have no limit check; `encode_buffer(None)` writes `00`.

**In the fork.** `decode_uint` (the `0xff` branch) and `decode_uint56` throw `UintOutOfRange` for a value > 2^53−1 (as in JS 3.x); `decode_buffer` of length 0 → `Some(vec![])` (JS 3.x), while `decode_optional_buffer` (`None` on empty) is for the protomux channel id (JS `c.optionalBuffer`); a buffer with a gigantic length is an error, not a panic. Unchanged: `decode_uint64` (it is `biguint64`, no limit), `c.frame` and `c.arraybuffer` are absent in Rust. Tests — `compact_encoding/strict_tests.rs`.

**Vectors.** The `c.uint` table above; `buffer` (`00` → empty `Buffer`), `uint64 ff0000000000002000` → error, `frame` length past the end → error.

**What of this the holesail path uses** (searched in the reference's `node_modules`). `c.buffer`: `dht-rpc/lib/io.js` (only under `if (value)`), `hyperdht/lib/messages.js` (`noise`, `payload`, record `value`s; `pluginRequest.value` — under `if (m.value)`). `null` does not reach `c.buffer` on this path: `router.js:98` `if (!reply || !reply.noise) return` comes before encoding, in the relay branches the fields are taken from the decoded message, `payload` is the result of `payload.encrypt(...)`. Relay `noise`/`payload` of length 0: the old version decodes `null` and writes `00`, the new one — an empty `Buffer` and also `00`; the bytes are the same. The rest: `protomux` — `c.optionalBuffer` for the id; `blind-relay` — `c.fixed32` for the token (`null` and a wrong length throw in both); `dht-rpc/lib/peer.js` — `c.ipv4Address`; `hyperdht/lib/messages.js:316` `c.array(c.raw)` did not change; `@hyperswarm/secret-stream` does not use compact-encoding.

**Unchanged:** `string` (lone surrogates → `efbfbd`), `fixed32/64`, `bool`, `array`, `biguint`, `float`, `ipv4/ipv6`; the full input table is 173 rows (old) / 174 (reference, +`uint32be`), the differences are only above and in the error text. Fuzz of receiving random buffers (140000 cases): "the old one returned a number, the reference threw" — `uint` 89 of 5000, `uint64` 1582, `uint56` 1768; no reverse cases.

## D13. protomux 3.10.1 → 3.12.1: length in a batch (beyond the list)

`bdc3b78` (3.12.1, `protomux/index.js:_onbatch`): was `state.end = state.start + len`; now `state.end = Math.min(state.start + len, end)`. A batch is the frame `00 00 <channel id> <len> <type> <body>`. For correct batches the bytes and parsing are the same (checked with 9 frames: two in a row `0000010300015203000152`, an empty frame, a plain frame — identical). If `len` is larger than the rest of the frame: the old version accepts the message and reads past the end, the channel's reply goes out as a **batch** `00000103000152`; the new one replies with a **plain** frame `01000152`, and a string longer than the data gives `Out of bounds` and the mux closes. The rest (`9cf17f2` id → `c.optionalBuffer`, `6044bba`, `e0fa729`) is not visible on the wire: the open frame `000101017000` matches.

**Rust:** `peeroxide-dht/src/protomux.rs` parsing of `CTRL_BATCH` requires `state.start + len <= end` and otherwise returns `InvalidFrame("batch item overflows frame")` — that is, it matches the stricter new behavior; the response to a malformed frame in the JS reference is a plain frame or a mux close, in Rust a frame error. No separate change is needed for valid batches.

**Vectors.** The batch frames above (`0000 01 14 00 03616263` etc.) → the reply bytes.

## D14. blind-relay 1.4.0 → 1.6.1 (beyond the list)

`0bfb303` (`blind-relay/index.js:_onunpair`). Was: an `unpair` by the token of an already paired pair closed (`destroy(PAIRING_CANCELLED)`) only the stream of the sender. Now: `link.destroyPair(err)` — **both** streams of the pair are torn down; closing the session also tears down the links. The format of `pair` messages (`flags` bitfield, `fixed32` token, `uint` id, `uint` seq) and `unpair` did not change, there are no timeouts. Visible on the relay server (the pair's second UDX stream is closed). The client did not change.

**Rust:** `peeroxide-dht/src/blind_relay.rs:BlindRelayServer::unpair` (returns `UnpairOutcome`) must close both streams for an active pair. The client (`BlindRelayClient::unpair`) needs no changes.

## D15. Packages with no wire changes

- `dht-rpc` 6.26.4 → 6.27.0: `ipv4Address` from `compact-encoding` instead of `compact-encoding-net`; the node id and address encoding are the same (`id('1.2.3.4',80)` = `7ec2783f74eca4b81ee86871fcce7f911efdbf58279696d7c92638ee097af502` in both; `ipv4Array` `020102030450000808080849c2` in both).
- `hypercore-crypto` 3.6.1 → 3.7.0: `namespace('hyperswarm/dht', [0..12])`, `hash`, `data`, `discoveryKey`, signatures — outputs are equal (`hash('abc') = bddd813c634239723171ef3fee98579b94964e3bb1cb3e427262c8c068d52319`). The first 8 bytes of `NS` for id 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 11, 12: `14d6d4b49214ab10`, `f1191cd5e67b10b5`, `0ea5f9f225bac23c`, `c9e2bffb41e679fc`, `36386adddf9f6fd6`, `ded293cd93fb395e`, `668e823edd5ce7f5`, `cda4d97105a38a41`, `1ce0cc184aa7d815`, `ff40419103cc0214`, `d3c98c1e7a84e405`, `4d7cd7a2bcad2c60`.
- `@hyperswarm/secret-stream` 6.9.1 → 6.9.2: only the `filterZeroByteMessages` flag (D5).

## Beyond the list: Rust differences from the reference

These are not version deltas (they differ from both the old and the new JS), but they are visible on the wire and matter for the vectors. Items 1-5 and 7 describe upstream peeroxide 1.7.3; the fork fixes them. The other items state the fork's current behavior.

1. **Upstream ignores SACK from JS.** JS writes an ACK with blocks like this: type `SACK`, `data_offset = 0`, the blocks right after byte 20 (`udx_write_header` writes `data_offset = 0`, `send_ack` puts `sacks` at the end). Rust (`stream.rs:process_incoming`) accepts SACK only with `data_offset > 0` (`has_sack = FLAG_SACK && data_offset > 0`) and decodes the `payload_offset()` slice; the blocks from JS are not read, only the cumulative `ack` works.
2. **SACK from upstream is truncated.** Rust writes `data_offset = 8·n` as a `u8` (`stream.rs:send_ack`: `sack_byte_len as u8`); `MAX_SACK_RANGES = 50` gives 400 bytes → 144 at `n = 50`, starting from `n = 32`. JS parses `header_len = data_offset (< payload_len)` and reads only part of the blocks.
3. **Unknown hyperdht command.** In the JS reference (both versions) a request with an unknown command gets the error reply `UNKNOWN_COMMAND` (code 1, `dht-rpc/index.js:_onrequest`); upstream 1.7.3 (`hyperdht.rs`, `_ =>`) is silent. For command 10 (PLUGIN) the reference is silent too (on a node with an `id`), D1 agrees.
   In the fork, `run_request_handler` (`hyperdht/node.rs`) answers an unknown command with an explicit `req.error(ERR_UNKNOWN_COMMAND)` (upstream drops the request implicitly); PLUGIN (10) — `release` (silence, as on a JS node with an `id`). Residual difference: on a JS node **without** an `id` (a localhost testnet) PLUGIN gets error 1, while the Rust table always has an `id` — PLUGIN is always silent; it does not occur on the holesail path. Tests — `peeroxide-dht/tests/hyperdht_unknown_command.rs`; the `PluginRequest` codec — `peeroxide-dht/src/hyperdht_plugin.rs` (tests `hyperdht_plugin_tests.rs`, three vectors).
4. **Drop threshold on silence** (D7): 61 s in upstream versus 13 s in JS (the fork: 13 s).
5. **Zero-window probe** (D4): absent in upstream (the fork has it).
6. **`seq` and window in ACK/HEARTBEAT.** `seq` = the next send `seq`, as in the reference. The window in Rust's ACK is the constant `DEFAULT_RWND`, in JS it is `get_recv_rwnd` (the difference remains); the JS check `ack ≤ seq` is not ported. One more difference: JS updates the send window `send_rwnd` only when the packet is "fresher" than the previous update (`udx.c`: `seq_compare(send_wl1, seq) < 0 || (send_wl1 == seq && seq_compare(send_wl2, ack) <= 0)`, the fields `send_wl1`/`send_wl2` are the seq and ack of the packet that gave the last window); Rust takes the window from every ACK without this check, likewise without the `ack ≤ seq` check (see above). The practical effect is only on reordered packets; this is not ported and does not matter on the holesail path.
7. **`seq` wraparound through 2^32** in `outgoing`/`recv_buf`/Recovery is broken in upstream; the fork uses the `Seq` type.
8. **Server unannounce: relays only, in parallel, 1.5 s cap.** JS (`lib/announcer.js:_unannounceAll`/`_unannounce`) sends FIND_PEER (for the token) and then UNANNOUNCE to the `_serverRelays[2]` nodes, i.e. those that accepted the last ANNOUNCE, with `Promise.allSettled` and no overall cap (each request waits 1000 ms × retries). Rust sends to the same set in the same order, with one cap for the whole phase: `UNANNOUNCE_DEADLINE` = 1.5 s (`peeroxide-dht/src/hyperdht/unannounce.rs`). Relays that dropped out of the set after a re-announce are withdrawn in the background (JS awaits them in `_update`). On the wire: a relay slower than the cap keeps the record until it expires; a node whose ANNOUNCE reply was lost, and new relays that accepted an ANNOUNCE when `close` came in the middle of a re-announce, do the same. Upstream withdraws the record from all ~20 nearest nodes after a fresh LOOKUP.

## Vector table: change → vector → how to capture from JS

| Change | Vector | How to capture from JS |
|---|---|---|
| D1 `NS_PLUGIN` | 32 bytes `6b8a2a86…a8d5`; plus the 12 first 8-byte prefixes from D15 | `require('hypercore-crypto').namespace('hyperswarm/dht', [10])`, or `hyperdht/lib/constants.js:NS.PLUGIN`; implemented in `testvectors/deltas-js/plugin.js` |
| D1 `pluginRequest` | `0178010200`, `017801020100`, `017801020102aabb` | `testvectors/deltas-js/plugin.js` (`compact-encoding` `c.encode(m.pluginRequest, …)`) |
| D1 PLUGIN with no reply | a command 10 request with an empty `value` → no reply | two JS nodes, `dht.request({command:10,value:null}, to)` — captured: `testvectors/deltas/plugin.json`. A node without an `id`: both versions `UNKNOWN_COMMAND`. A persistent node with an `id` (testnet nodes `nodes[1..]`): 6.34.0 is silent for an empty `value` and for an unregistered plugin (`REQUEST_TIMEOUT`), and replies for a registered `x`; 6.30.0 replies `UNKNOWN_COMMAND` (no PLUGIN branch) |
| D2/D3 empty MESSAGE | `ff01080007000000000040000000000000000000` | `testvectors/deltas-js/udxcap.js` (`stream.trySend(Buffer.alloc(0))`, the peer is a Node UDP socket) |
| D3 HEARTBEAT | `ff01200007000000000040000000000000000000` | `testvectors/deltas-js/udxcap.js` (`stream.setKeepalive(300)`) |
| D3 reply to HEARTBEAT | a 20-byte ACK for a fed 0x20 | modeled on `udxzwp.js`: feed a 0x20 from a UDP peer, capture the reply — captured: `testvectors/deltas/udx.json`, `results.{new,old}.heartbeatReply` |
| D4 ZWP | HEARTBEAT at 0.1 s, +1 s, +2 s, +2 s (reference); DATA at 0.1 s (old) | `testvectors/deltas-js/udxzwp.js` on two versions of udx-native |
| D6 strict SACK | ACK with blocks: valid; `end == seq`; `start < remote_acked`; `end > seq`; `start >= end` → which packets are retransmitted | a raw UDP peer sends ACK packets, capture JS's retransmits (`udxrto.js` as a starting point) — captured: `testvectors/deltas/udx.json`, `results.{new,old}.strictSack_*` |
| D7 RTO | retransmits at 1, 3, 5, 7, 9, 11 s, `ETIMEDOUT` at 13 s (both versions on silence) | `testvectors/deltas-js/udxrto.js` on two versions |
| D7 RTT/TLP on localhost | the interval to the probe packet `1·2 + 2` ms | the peer answers DATA with an ACK; capture the interval between a DATA and its retransmit — partly captured: `testvectors/deltas/udx-probes.json`, `results.{new,old}.tlp` (20 attempts per version). No retransmit within 700 ms: new 0–2 of 20, old 6–8 of 20 (an RTT sample of 0 is read as "no sample", TLP at 1000 ms); the file has the boolean `noRetransmitWithin700msInAtLeast20pctOfTrials` (new false, old true). The exact interval `1·2 + 2` ms is not captured: the new one's retransmit goes out in <20 ms |
| D8 ACK | N DATA → N ACKs, with SACK at gaps | a UDP peer sends DATA to a JS stream, capture the replies — captured: `testvectors/deltas/udx.json`, `results.{new,old}.ackPerData` |
| D9 MTU probe | DATA with `data_offset != 0`, the datagram length equals `wanted_size` | a JS stream writes a lot of data to a UDP peer, capture the packets with `header[3] != 0` — captured: `testvectors/deltas/udx-probes.json`, `results.{new,old}.mtuProbe`: 10 probes, datagrams 1204…1472 bytes in steps of 32, `data_offset` = zero padding; `data_offset` of the new one is 32, then 64, the last 44, of the old one 32, the last 12; payload 1152…1408 (new), 1152…1440 (old) |
| D9 `change_remote` | packets after the switch (address, `remote_id`, MTU) | two UDX streams and `changeRemote` — captured: `testvectors/deltas/udx-probes.json`, `changeRemote` and `changeRemoteInflight` (both versions identical): after the switch DATA goes to the new port with `remote_id` 9, nothing to the old peer; with an unacknowledged DATA seq 1 in flight it stays with the old peer, the next seq 2 goes to the new one |
| D11 reply port | source port of the handshake reply | two JS nodes — captured: `testvectors/deltas/handshake-socket.json` (which socket is announced: on loopback neither version announces the puncher socket) and `testvectors/probes-js/fork-d11-probe-js.mjs` (output `fork-d11-probe-js.out.txt`): the source port for open, fw without LAN and fw with a LAN shortcut × direct, relay, second, hyperdht 6.34.0 (the table is in D11). No real pair on one LAN was captured; the Rust server takes a reduced LAN shortcut (see the mechanism paragraph of D11) |
| D12 `c.buffer` / `c.uint` | the `c.uint` table from D12; `00` → empty Buffer; `ff0000000000002000` → error | `compact-encoding` 3.5.2 from the reference's `node_modules`, values from the table |
| D13 batch | input `0000 01 14 00 03616263` → reply `01000152` (reference) | `protomux` 3.12.1 with a fake stream, channel `c.string`, the responder sends `'R'` — captured: `testvectors/deltas/codec.json`, `d13.{new,old}.frames` |
| D14 unpair | closing both streams of the pair | a relay — captured: `testvectors/deltas/blind-relay.json` (a server without a network). New 1.6.1: an unpair from the initiator and from the responder closes both streams of the pair (`pairingsCancelled` 1, `streamsClosed` 2); old 1.4.0: only the sender's stream; the initiator session dropping — only its stream in both |

## Open points

- **Old zero-window probe**: the capture shows DATA at 102 ms and no packets over the next 7 s (neither a retransmit nor a probe); per the source the old probe is DATA. The reason for the missing retransmit is not found.
- What goes out right after `change_remote`, `udx_stream_connect` on a connected stream, the contents of `udx_rack_reo_timeout`, the send time of the old DESTROY: not examined.
- A real exchange through a blind relay and the "relay → direct" scenario as a whole (D2, D9) are not captured. The source port of the handshake reply (D11) is captured on loopback for three kinds of server and three cases (see D11); no real pair on one LAN.
- Not examined: libudx tests `test/stream-relay-firewall-source.c` and `stream-retransmit-refreshes-header.c`, the function with the line `inflight -= pkt->size` (`ae8bff7`); `c.json`/`ndjson`, `lexint`, shortened ipv6 forms on garbage inputs in compact-encoding.
- The branch "empty `noise` from a peer" (`router.js:94`) is described from the source; there is no separate hyperdht capture for it; protomux `6044bba` (an exception in the channel handshake) is not reproduced; the exact UDX packets of a live relay for `0bfb303` are not captured (D14 is captured on a server without a network).
