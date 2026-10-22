// JS: libudx/src/udx.c
//! Stream constants: window, MTU, RTO, SACK and RTT limits; see also `types.rs`, `inner.rs`.

use super::*;

// ── Constants ────────────────────────────────────────────────────────────────

/// Default receive window size (4 MB, matches C libudx).
pub(crate) const DEFAULT_RWND: u32 = 4_194_304;

/// Base of the adapter write budget — mirrors `UDX_HIGH_WATERMARK` (C: udx.c:47).
pub(crate) const WRITE_HIGH_WATERMARK: usize = 262_144;

/// IPv4 + UDP + udx header overhead — mirrors `UDX_IPV4_HEADER_SIZE` (udx.h:15).
pub(crate) const IPV4_OVERHEAD: usize = 20 + 8 + HEADER_SIZE;

/// Same for IPv6 — mirrors `UDX_IPV6_HEADER_SIZE` (udx.h:16).
pub(crate) const IPV6_OVERHEAD: usize = 40 + 8 + HEADER_SIZE;

/// Payload at the base MTU, IPv4; `mtu` is the IP packet size.
pub(crate) const MAX_PAYLOAD: usize = MTU_BASE - IPV4_OVERHEAD;

/// Starting MTU (bytes) — matches C libudx `UDX_MTU_BASE`.
pub(crate) const MTU_BASE: usize = 1200;

/// Maximum MTU (bytes) — matches C libudx `UDX_MTU_MAX`.
pub(crate) const MTU_MAX: usize = 1500;

/// Probe size increment (bytes).
pub(crate) const MTU_STEP: usize = 32;

/// Failed probes before settling on current MTU.
pub(crate) const MTU_MAX_PROBES: u32 = 3;

/// Initial retransmission timeout (ms) — matches C libudx.
pub(super) const INITIAL_RTO_MS: u32 = 1_000;

/// Maximum retransmission timeout (ms) — matches C libudx `UDX_RTO_MAX_MS`.
pub(super) const MAX_RTO_MS: u32 = 30_000;

/// Maximum consecutive RTO timeouts before stream failure.
pub(super) const MAX_RTO_TIMEOUTS: u32 = 6;

/// Maximum number of SACK ranges to include in an ACK packet.
pub(super) const MAX_SACK_RANGES: usize = 50;

/// Maximum accepted RTT sample (ms) — prevents outlier corruption.
pub(super) const MAX_RTT_MS: u32 = 30_000;

/// Far-future duration used to effectively disable a timer.
pub(super) const FAR_FUTURE: std::time::Duration = std::time::Duration::from_secs(86_400);
