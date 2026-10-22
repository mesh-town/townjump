//! Snapshot of the sender state for diagnostics: `UdxStream::stats`, `UdxAsyncStream::stats`.
//! Related — `inner.rs`.

use super::*;

/// Snapshot of the sender state: read-only field access, stream behavior does not change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UdxStats {
    /// Smoothed RTT, ms.
    pub srtt_ms: u32,
    /// Current retransmission timeout, ms.
    pub rto_ms: u32,
    /// Congestion window, packets.
    pub cwnd: u32,
    /// Slow-start threshold, packets.
    pub ssthresh: u32,
    /// In flight (without SACK and without losses), packets.
    pub inflight: u32,
    /// Pacing rate from BBR, bytes per ms.
    pub pacing_bytes_per_ms: u32,
    /// Cumulative delivered packets.
    pub delivered: u32,
    /// Cumulative lost packets.
    pub lost: u32,
    /// BBR mode as the C code `UDX_BBR_STATE_*`: 0 Startup, 1 Drain, 2 ProbeBw, 3 ProbeRtt.
    pub mode: u32,
    /// 1 if BBR found a bandwidth plateau at startup (`full_bw_reached`), otherwise 0.
    pub full_bw_reached: u32,
    /// 1 if the app-limited mark of the current rate sample is not 0 (`congestion.app_limited != 0`), otherwise 0.
    pub app_limited: u32,
    /// BBR bandwidth estimate (`bw`, packets/ms) × packet payload, bytes per ms; saturates at `u32::MAX`.
    pub bw_bytes_per_ms: u32,
    /// Cumulative number of RTO timer fires with a non-empty `outgoing` (`on_rto_fire`), count.
    pub rto_fires: u32,
    /// Cumulative number of packets retransmitted (RTO burst and fast retransmission, `resend_one`), count.
    pub resent: u32,
    /// BBR min RTT (`min_rtt_ms`), ms; at least 1 (before the first measurement — 1).
    pub min_rtt: u32,
}

/// `bw` (packets per ms) × `mss` (payload bytes) -> bytes per ms; NaN and negative — 0, above `u32::MAX` — `u32::MAX`.
pub(crate) fn bw_bytes_per_ms(bw: f64, mss: usize) -> u32 {
    let v = bw * mss as f64;
    if v.is_nan() || v < 0.0 {
        0
    } else {
        v.min(u32::MAX as f64) as u32
    }
}

impl StreamInner {
    pub(crate) fn stats(&self) -> UdxStats {
        UdxStats {
            srtt_ms: self.srtt,
            rto_ms: self.rto,
            cwnd: self.congestion.cwnd,
            ssthresh: self.congestion.ssthresh,
            inflight: self.inflight(),
            pacing_bytes_per_ms: self.congestion.pacing_bytes_per_ms,
            delivered: self.congestion.delivered,
            lost: self.congestion.lost,
            mode: self.congestion.bbr().mode() as u32,
            full_bw_reached: u32::from(self.congestion.bbr().full_bw_reached()),
            app_limited: u32::from(self.congestion.app_limited != 0),
            bw_bytes_per_ms: bw_bytes_per_ms(self.congestion.bbr().bw(), self.max_payload()),
            rto_fires: self.rto_fires,
            resent: self.outgoing.resent(),
            min_rtt: self.congestion.bbr().min_rtt_ms(),
        }
    }
}

impl UdxStream {
    /// Snapshot of the sender state (RTT, window, pacing, counters) for diagnostics.
    pub fn stats(&self) -> UdxStats {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.stats()
    }
}

#[cfg(test)]
#[path = "stats_tests.rs"]
mod tests;
