//! Packet retransmission and Karn protection for BBR's `rtt` (C `udx.c:1342-1344`, `udx_rate.c:8-15`).
use super::*;

impl CongestionController {
    /// Stream min RTT (`rack.rtt_min_ms`) for Karn protection in `on_ack`.
    pub(crate) fn set_rtt_min(&mut self, v: Option<u32>) {
        self.rtt_min_ms = v;
    }

    /// Packet retransmission (C `udx__rate_pkt_sent`): the flight start is set only if it did not exist yet
    /// (`udx_rate.c:8-15`, on retransmission `stream->seq != remote_acked`); the `retransmitted` mark is for Karn protection.
    pub(crate) fn on_packet_resent(&mut self, now: Instant) -> rate::PacketRateInfo {
        if self.first_sent_ts.is_none() {
            self.first_sent_ts = Some(now);
            self.delivered_ts = Some(now);
        }
        self.rate_info(now, true)
    }

    /// Snapshot of the delivery state for the packet being sent now.
    pub(super) fn rate_info(&self, now: Instant, retransmitted: bool) -> rate::PacketRateInfo {
        let info = rate::RateState::on_packet_sent(
            self.delivered,
            self.delivered_ts.unwrap_or(now),
            self.first_sent_ts.unwrap_or(now),
            now,
            self.app_limited > 0,
        );
        rate::PacketRateInfo {
            retransmitted,
            ..info
        }
    }
}
