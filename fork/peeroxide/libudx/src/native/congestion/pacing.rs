//! Token-bucket pacer for BBR send-rate gating (port of C libudx `tb_available`).
// JS: libudx/src/udx.c: stream_may_send, _send_packet, update_pacing_time

use std::time::{Duration, Instant};

/// Initial allowance in bytes: C `UDX_INIT_PACING_RATE`.
const INIT_TOKENS: u32 = 25_000;

/// Sending is allowed while any allowance is left; every whole millisecond replaces the
/// allowance with `elapsed_ms * rate` (no burst cap, as in C).
#[derive(Debug, Clone)]
pub(crate) struct TokenBucket {
    available: u32,
    last_refill: Option<Instant>,
    rate_bytes_per_ms: u32,
}

impl TokenBucket {
    pub(crate) fn new() -> Self {
        Self {
            available: INIT_TOKENS,
            last_refill: None,
            rate_bytes_per_ms: 0,
        }
    }

    pub(crate) fn set_rate(&mut self, bytes_per_ms: u32) {
        self.rate_bytes_per_ms = bytes_per_ms;
    }

    /// C `update_pacing_time`: whole milliseconds since the last refill; the remainder stays.
    pub(crate) fn refill(&mut self, now: Instant) {
        let Some(last) = self.last_refill else {
            self.last_refill = Some(now);
            return;
        };
        let elapsed_ms = now.saturating_duration_since(last).as_millis() as u32;
        if elapsed_ms == 0 || self.rate_bytes_per_ms == 0 {
            return;
        }
        self.available = elapsed_ms.saturating_mul(self.rate_bytes_per_ms);
        self.last_refill = Some(last + Duration::from_millis(u64::from(elapsed_ms)));
    }

    /// C `_send_packet`: the allowance goes to zero, not below. `true` if any was left.
    pub(crate) fn try_consume(&mut self, bytes: u32, now: Instant) -> bool {
        self.refill(now);
        let had_allowance = self.available > 0;
        self.available = self.available.saturating_sub(bytes);
        had_allowance
    }

    /// Zero while any allowance is left (C `stream_may_send`) or no rate is set yet;
    /// otherwise the rest of the current millisecond. Call `refill` first.
    pub(crate) fn delay_for(&self, now: Instant) -> Duration {
        if self.rate_bytes_per_ms == 0 || self.available > 0 {
            return Duration::ZERO;
        }
        let since = self
            .last_refill
            .map_or(Duration::ZERO, |l| now.saturating_duration_since(l));
        Duration::from_millis(1).saturating_sub(since)
    }

    /// Test: shifts `last_refill` back by `by` (emulates the passage of time).
    #[cfg(test)]
    pub(crate) fn rewind(&mut self, by: Duration) {
        self.last_refill = self.last_refill.map(|l| l - by);
    }

    /// Test: remaining allowance in bytes.
    #[cfg(test)]
    pub(crate) fn available(&self) -> u32 {
        self.available
    }

    pub(crate) fn reset(&mut self, now: Instant) {
        self.available = INIT_TOKENS;
        self.last_refill = Some(now);
    }
}

#[cfg(test)]
#[path = "pacing_tests.rs"]
mod tests;
