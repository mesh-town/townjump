# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- The "stream failed: exceeded max RTO timeouts" warning carries local_id, remote_addr, unacked, closing and age_ms.
- Loss detection follows the send order (RACK with `reo_wnd = 0`): a packet below the highest SACK is lost only if it was sent earlier than the latest delivered SACKed packet that was never re-sent, and a hole is re-sent at most once per RTT; an RTT sample is not taken from a SACKed packet. Before this every ACK re-marked and re-sent the same holes, and after a burst loss the storm of re-sends overran the receiver's UDP buffer and stalled the transfer (which is why `WRITE_HIGH_WATER` had to stay at 512 KiB).
- `UdxAsyncStream::poll_write` no longer waits for the ACK of the previous write: it accepts data while the payload without ACK or SACK is below `UdxAsyncStream::WRITE_HIGH_WATER` (256 KiB, as C `UDX_HIGH_WATERMARK`) plus `min(cwnd * mss, peer receive window)` (with any room above zero it accepts up to 64 KiB, so up to 64 KiB - 1 over); a SACK frees its bytes at once, not only the cumulative ACK; `poll_flush` and `poll_shutdown` wait for the ACK of everything accepted.
- Dropping a `UdxAsyncStream` before the stream has ended (our FIN acknowledged and the peer's FIN read) now sends DESTROY, so the peer's reader fails with `ConnectionReset` instead of a silent timeout; a peer DESTROY surfaces as `ConnectionReset` on read, a peer FIN stays EOF. New `UdxAsyncStream::abort(self)` always sends DESTROY.
- Sender bookkeeping is O(1): the in-flight count, SACKed/lost counts and queued bytes are kept as counters instead of walking the in-flight map and the send queue per packet and per ACK. Measured on loopback (256 MiB through the tunnel, medians on a noisy machine): about 5-30% faster sending than before at the same 512 KiB bound.
- Loss detection walks the in-flight map only up to the highest SACKed packet.
- The sender now fills the whole congestion window: packets sent in the current wake-up were counted in flight twice, so only half of the window was used.

### Fixed

- RACK detects reordering as C `reordering_seen` (a never-resent packet delivered below the highest delivered one); once seen, `reo_wnd` stays `min(rtt_min/4, srtt)` in Recovery/Loss and with >= 3 SACKs, so reordered packets on a jittery path are no longer re-sent as lost.
- HEARTBEAT is answered with an ACK and processing of that packet stops (no window, ack, SACK or data handling); the once-a-second keepalive HEARTBEAT is removed.
- ACK and HEARTBEAT packets carry `seq` = the first unsent sequence number instead of 0.
- SACK blocks are written right after the 20-byte header with `data_offset = 0` (up to 50 blocks) and read by the C rule `header_len = data_offset` if `0 < data_offset < payload_len`, else `payload_len`; blocks outside `remote_acked <= start < end <= seq` (mod 2^32) are skipped.
- RTO: `rto` no longer doubles, the next timer is `rto * 2`, the 7th consecutive expiry fails the stream with `TimedOut` (about 13 s instead of about 61 s); a zero RTT sample counts as 1 ms; the RTO timer is re-armed after an ACK only when the cumulative ACK advances.
- A non-zero send window smaller than one packet admits one packet.

### Added

- Closed streams linger as tombstones for clamp(14·rto, 20 s, 60 s): a late DATA or END from the peer gets a DESTROY, or an ACK when both ENDs were exchanged, at most once per rto.
- Zero-window probe: a 20-byte HEARTBEAT right after a write into an idle stream with window 0, then after `rto`, then every `rto * 2`, until the window opens.

## [1.7.3](https://github.com/Rightbracket/peeroxide/compare/libudx-v1.7.2...libudx-v1.7.3) - 2026-07-12

### Other

- full documentation audit (rustdoc + mdBook) ([#28](https://github.com/Rightbracket/peeroxide/pull/28))

## [1.7.2](https://github.com/Rightbracket/peeroxide/compare/libudx-v1.3.1...libudx-v1.7.2) - 2026-07-12

### Other

- NAT holepunch hardening + blind-relay server implementation ([#27](https://github.com/Rightbracket/peeroxide/pull/27))

### Fixed

- Fixed a `process_incoming` ordering bug: the relay fast-path (packet forwarding once `UdxStream::relay_to` is configured) was checked before the firewall-hook gate, permanently starving hook-based 4-tuple address adoption whenever `relay_to` was pre-wired on both sides of a pairing before either side's first packet arrived. The relay-forwarding check now runs after the firewall-hook gate.

## [1.3.1](https://github.com/Rightbracket/peeroxide/compare/libudx-v1.3.0...libudx-v1.3.1) - 2026-05-14

### Other

- release ([#20](https://github.com/Rightbracket/peeroxide/pull/20))

## [1.3.0](https://github.com/Rightbracket/peeroxide/compare/libudx-v1.2.0...libudx-v1.3.0) - 2026-05-14

### Other

- release ([#19](https://github.com/Rightbracket/peeroxide/pull/19))

## [1.2.0](https://github.com/Rightbracket/peeroxide/compare/libudx-v1.1.0...libudx-v1.2.0) - 2026-05-01

### Changed

- `UdxSocket` is now a cheap-clone `Arc` handle (`UdxSocketInner` holds all state internally). All clones share the same underlying socket; the recv loop is only torn down when the last clone is dropped. `UdxSocket::close(self)` consuming signature is unchanged. ([#12](https://github.com/Rightbracket/peeroxide/pull/12))

## [1.1.0](https://github.com/Rightbracket/peeroxide/compare/libudx-v1.0.1...libudx-v1.1.0) - 2026-04-28

### Changed

- `UdxError` marked `#[non_exhaustive]` to allow future variants without a breaking change. ([#10](https://github.com/Rightbracket/peeroxide/pull/10))

## [1.0.1](https://github.com/Rightbracket/peeroxide/compare/libudx-v1.0.0...libudx-v1.0.1) - 2026-04-26

### Changed

- Added `#[forbid(unsafe_code)]` and `[package.metadata.docs.rs]` configuration.
- Fixed doc example to use `?` instead of `.unwrap()`.
- Increased network simulation test timeout from 120 s to 300 s for slower CI runners.
- Updated repository URL, badges, and README links after repo transfer to the `Rightbracket` org.

## [1.0.0](https://github.com/Rightbracket/peeroxide/releases/tag/v1.0.0) - 2026-04-25

Initial release. Pure Rust implementation of the UDX protocol with BBR
congestion control, wire-compatible with the existing Node.js network.

### Added

- Pure Rust UDX protocol implementation (replaced C FFI bindings)
- BBR congestion control (faithful port of C `udx_bbr.c`)
- Reliability: cumulative ACK, SACK, retransmission with RTO, fast retransmit
- RTT estimation (Jacobson/Karels per RFC 6298)
- Rate sampling for BBR bandwidth estimation
- Token bucket pacing
- MTU probing (base 1200, max 1500, step 32)
- Relay packet forwarding (header rewriting, DESTROY propagation)
- `UdxAsyncStream`: `AsyncRead + AsyncWrite + Unpin` adapter for tokio
- Multiplexing: multiple streams per socket with independent congestion state
- Heartbeat keepalive (1s interval)
- Graceful shutdown with buffered write drain

### Tested

- 53 unit tests passing
- 74 integration and network simulation tests (loss, delay, jitter, reorder, MTU clamping)
- Golden byte fixtures verified against Node.js libudx reference implementation

### Dependencies

- `tokio` — async runtime
- `tracing`, `thiserror` — logging and error handling

### Compatibility

- Wire-compatible with Node.js libudx
- Rust edition 2024, MSRV 1.85
- Dual-licensed: MIT OR Apache-2.0
