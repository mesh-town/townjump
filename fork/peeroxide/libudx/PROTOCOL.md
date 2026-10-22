# UDX Protocol Reference

UDX is a lightweight reliable transport protocol built on UDP. There is no formal
specification document — the C source code at
[holepunchto/libudx](https://github.com/holepunchto/libudx) is the reference.
This document describes the wire format and protocol behavior as implemented by
this crate.

## Packet Header (20 bytes)

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|  Magic (0xFF) |  Version (1)  |  Type/Flags   |  Data Offset  |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                     Remote Stream ID (LE)                     |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                     Receive Window (LE)                       |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                     Sequence Number (LE)                      |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                     ACK Number (LE)                           |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

| Offset | Size | Field | Description |
|--------|------|-------|-------------|
| 0 | 1 | magic | Always `0xFF` |
| 1 | 1 | version | Always `1` |
| 2 | 1 | type | Packet type flags (bitfield) |
| 3 | 1 | data_offset | Bytes of extension data between header and payload (SACK, MTU padding) |
| 4-7 | 4 | remote_id | Remote stream identifier (little-endian u32) |
| 8-11 | 4 | recv_window | Receiver's advertised window in bytes (LE u32) |
| 12-15 | 4 | seq | Sequence number (LE u32) |
| 16-19 | 4 | ack | Acknowledgment number (LE u32) |

All multi-byte fields are little-endian.

## Packet Type Flags (byte 2)

Flags are a bitfield. Multiple can be set simultaneously (e.g., `DATA | END`
for the last data packet, or `DATA | SACK`).

| Flag | Value | Description |
|------|-------|-------------|
| `DATA` | `0x01` | Data payload present |
| `END` | `0x02` | End of stream (write-side FIN) |
| `SACK` | `0x04` | Selective acknowledgment blocks follow the header |
| `MESSAGE` | `0x08` | Unreliable datagram (no retransmission) |
| `DESTROY` | `0x10` | Immediate stream teardown |
| `HEARTBEAT` | `0x20` | Zero-window probe; a received HEARTBEAT is answered with an ACK |

## SACK Format

An ACK carrying SACK blocks sets `SACK`, writes `data_offset = 0` and places up
to 50 blocks right after the 20-byte header (packet length `20 + 8n`), as the C
reference does. A block is a pair `(start, end)` of sequence numbers encoded as LE
u32 values (8 bytes), meaning `[start, end)`.

On receive the block area is `header_len` bytes, where `header_len = data_offset`
if `0 < data_offset < payload_len`, otherwise `payload_len`; the number of
blocks is `header_len / 8` (a trailing partial block is dropped). The older
Rust layout (`data_offset = 8n = payload_len`) is read by the same rule.

A block is applied only if `remote_acked <= start < end <= seq` (comparison
mod 2^32, `seq` = sequence number of the first unsent packet); an invalid
block is skipped. The cumulative ACK of the same packet is applied first.

`seq` of every ACK and HEARTBEAT is the sequence number of the first packet not
yet sent (the head of the send queue, otherwise the next sequence number to
assign); the window field is always `DEFAULT_RWND`.

## HEARTBEAT and Zero-Window Probe

- A received HEARTBEAT is answered with an ACK (with the current SACK blocks)
  and processing of that packet stops: its window, ack and SACK fields are
  ignored. There is no periodic keepalive timer.
- Zero-window probe (ZWP): while the peer advertises window 0 and unsent data
  is queued with nothing in flight, the sender emits a 20-byte HEARTBEAT (its
  `seq` is not consumed). The first probe goes out right after the write into
  an idle stream, the next one after `rto`, then every `rto * 2`, without
  limit. When the window becomes non-zero the probing stops.
- A non-zero window smaller than one packet still admits one packet.

## Stream Multiplexing

Multiple streams share a single UDP socket. Incoming packets are demultiplexed
by the `remote_id` field (bytes 4-7), which maps to the receiver's local stream ID.

## Congestion Control: BBR

UDX uses BBR (Bottleneck Bandwidth and RTT) congestion control.

### State Machine

```
STARTUP --[bw plateau 3 rounds]--> DRAIN --[inflight <= BDP]--> PROBE_BW
                                                                    |
                                                     every 10s ----+
                                                                    v
                                                               PROBE_RTT
                                                     (200ms, cwnd=4) |
                                                                    |
                                                               back to
                                                               PROBE_BW
```

| State | Pacing Gain | cwnd Gain | Exit Condition |
|-------|-------------|-----------|----------------|
| STARTUP | 2.885 (2/ln2) | 2.0 | BW hasn't grown 25% for 3 rounds |
| DRAIN | 0.347 (1/2.885) | 2.0 | Inflight <= BDP |
| PROBE_BW | cycles [1.25, 0.75, 1.0x6] | 2.0 | Cycles each RTT; PROBE_RTT every 10s |
| PROBE_RTT | 1.0 | -- (cwnd=4 pkts) | After 200ms + 1 RTT with inflight <= 4 |

### Loss Recovery (CA states)

| State | Value | Description |
|-------|-------|-------------|
| CA_OPEN | 1 | Normal operation |
| CA_RECOVERY | 2 | Fast recovery after SACK-detected losses |
| CA_LOSS | 3 | RTO-triggered loss recovery |

Initial values: `cwnd = 10 packets`, `ssthresh = 0xFFFF`.

## RTT Estimation (Jacobson/Karels)

```
First sample:
  srtt = rtt
  rttvar = rtt / 2

Subsequent samples:
  delta = |srtt - rtt|
  rttvar = (3 * rttvar + delta) / 4
  srtt = (7 * srtt + rtt) / 8

RTO = max(srtt + 4 * rttvar, 1000ms)
RTO capped at 30,000ms
A zero RTT sample counts as 1 ms.
```

Every RTO expiry with data in flight increments a counter and retransmits; the
next expiry is scheduled `rto * 2` ahead, while `rto` itself does not grow.
The 7th consecutive expiry fails the stream with `TimedOut` (about 13 s with
the initial `rto` of 1 s). The counter is reset only when the cumulative ACK
advances. The RTO timer is re-armed after an ACK only when the cumulative ACK
advances (cleared when everything is acknowledged). Tail loss probe (TLP) is
not implemented.

## MTU Path Discovery

UDX probes the path MTU by sending progressively larger packets.

| Constant | Value | Description |
|----------|-------|-------------|
| MTU_BASE | 1200 | Starting MTU (bytes) |
| MTU_MAX | 1500 | Maximum MTU (bytes) |
| MTU_STEP | 32 | Probe size increment |
| MTU_MAX_PROBES | 3 | Failed probes before settling |

State machine: `BASE --> SEARCH --> SEARCH_COMPLETE`.

Probes reuse the `data_offset` byte to indicate padding size. The payload
scales from 1180 bytes (MTU_BASE - header) up to 1480 bytes (MTU_MAX - header).

## Rate Sampling and Pacing

BBR uses rate sampling to estimate bottleneck bandwidth:

- Each sent packet records `delivered` count and `delivered_ts` timestamp
- On ACK, delivery rate = `(delivered_now - delivered_at_send) / (now - delivered_ts_at_send)`
- Windowed max filter tracks peak bandwidth over recent rounds
- Token bucket paces sends: `pacing_rate = bw * mss * pacing_gain * 0.99`

## Relay (Packet Forwarding)

`relay_to` creates a fast-path packet relay between two streams on the same runtime:

1. When a packet arrives on the source stream, the relay rewrites only the
   `remote_id` field (bytes 4-7) to the destination's remote ID
2. The packet is forwarded to the destination's remote address via its UDP socket
3. If the DESTROY flag is set on the relayed packet, the source stream is closed
4. The relay does not participate in congestion control or acknowledgment

Both streams must be on the same `UdxRuntime`.

## Protocol Constants

| Constant | Value | Description |
|----------|-------|-------------|
| HEADER_SIZE | 20 | Fixed header size (bytes) |
| MAGIC | 0xFF | Magic byte |
| VERSION | 1 | Protocol version |
| DEFAULT_RWND_MAX | 4,194,304 | Default max receive window (~4MB) |
| RTO_MAX_MS | 30,000 | Maximum RTO |
| MAX_RTO_TIMEOUTS | 6 | Consecutive RTO expiries tolerated; the 7th fails the stream |
| MAX_SACK_RANGES | 50 | SACK blocks per ACK |

## Known Limitations

This implementation has the following known simplifications relative to the
C reference:

- `detect_losses()` selects `highest_sacked` via raw u32 `.max()`, not
  wrap-safe across sequence number wraparound in very long-lived streams
- `BTreeMap::split_off(&ack)` in cumulative ACK processing uses raw u32
  ordering (same wraparound concern)
- Async shutdown drains queued packets but does not wait for ACK/retransmit
  completion after drop
- `check_app_limited()` always evaluates with `retransmit_pending = false`
- Loss detection uses simplified SACK-gap rule, not full RACK timing logic
- Tail loss probe (TLP) is not implemented; the window field in ACK/HEARTBEAT is
  the constant `DEFAULT_RWND` (C reference: current receive window)
- `UdxError::Uv(i32)` exists for API compatibility but is never emitted
  by the native backend (only `Io` and `StreamClosed` are used)

## Reference

- C libudx source: <https://github.com/holepunchto/libudx>
- Node.js udx-native: <https://github.com/holepunchto/udx-native>
- BBR paper: <https://dl.acm.org/doi/10.1145/3009824>
- RFC 6298 (TCP RTO): <https://tools.ietf.org/html/rfc6298>
- RFC 2018 (TCP SACK): <https://tools.ietf.org/html/rfc2018>
