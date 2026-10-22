// JS: libudx/src/udx.c:ack_packet (rack_rtt, rack_time_sent: 1342-1349; rtt_min: 1311-1314; reordering: 1352-1357)
//! RACK clock: send time of the latest delivered packet and the RTT of the last delivery (RFC 8985 §6.2 step 2).

use super::seq::Seq;
use std::time::{Duration, Instant};

/// C `rack_time_sent` (`sent`), `rack_rtt` (`rtt`), `udx_rtt_min` without the 300 s window (`rtt_min_ms`; `None` — no measurements yet).
#[derive(Debug, Default, Clone, Copy)]
pub(super) struct Rack {
    pub(super) sent: Option<Instant>,
    pub(super) rtt: Duration,
    pub(super) rtt_min_ms: Option<u32>,
    /// C `rack_fack`: one above the highest delivered number (`None` — nothing delivered yet; C starts from 0).
    pub(super) fack: Option<Seq>,
    /// C `reordering_seen`: a never-resent packet was delivered below the highest delivered one; never reset for the life of the stream.
    pub(super) reordering_seen: bool,
}

impl Rack {
    /// Delivery of packet `seq` (call before `on_delivered`). C 1352-1357: above `fack` moves it, below it and not
    /// resent — reordering seen; compared modulo 2^32 (`Seq` order).
    pub(super) fn on_acked_seq(&mut self, seq: Seq, retransmitted: bool) {
        let next = Seq(seq.0.wrapping_add(1));
        match self.fack {
            Some(fack) if next < fack => self.reordering_seen |= !retransmitted,
            Some(fack) if next == fack => {}
            _ => self.fack = Some(next),
        }
    }

    /// Packet delivery (SACK or cumulative ACK). A retransmission faster than `rtt_min` is a reply to a previous copy
    /// (C 1342: `!retransmitted || rtt >= rtt_min`): it does not move the clock.
    pub(super) fn on_delivered(&mut self, time_sent: Instant, retransmitted: bool, now: Instant) {
        let rtt = now.saturating_duration_since(time_sent);
        let rtt_ms = u32::try_from(rtt.as_millis()).unwrap_or(u32::MAX);
        if retransmitted && self.rtt_min_ms.is_none_or(|min| rtt_ms < min) {
            return;
        }
        self.sample_min(rtt_ms, retransmitted);
        self.rtt = rtt;
        self.sent = self.sent.max(Some(time_sent));
    }

    /// Min RTT — from first-transmission packets only; 0 ms counts as 1 (C 1311).
    fn sample_min(&mut self, rtt_ms: u32, retransmitted: bool) {
        if retransmitted {
            return;
        }
        let rtt_ms = rtt_ms.max(1);
        self.rtt_min_ms = Some(self.rtt_min_ms.map_or(rtt_ms, |min| min.min(rtt_ms)));
    }
}

#[cfg(test)]
#[path = "rack_tests.rs"]
mod tests;
