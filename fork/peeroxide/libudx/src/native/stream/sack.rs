//! Parsing of incoming SACK blocks and applying them (D6: block validation, JS udx.c: process_sacks).
use super::*;
use crate::native::congestion::rate::PacketRateInfo;
use crate::native::header::decode_sack;

/// D6: a block is valid if `remote_acked <= start < end <= seq` (mod 2^32).
pub(super) fn sack_block_valid(remote_acked: u32, seq: u32, start: u32, end: u32) -> bool {
    seq_diff(start, remote_acked) >= 0 && seq_diff(end, start) > 0 && seq_diff(seq, end) >= 0
}

/// Length of the SACK block area (a multiple of 8): `data_offset` if it lies inside the payload, otherwise the whole payload.
pub(super) fn sack_block_bytes(data_offset: u8, payload_len: usize) -> usize {
    let offset = usize::from(data_offset);
    let header_len = if offset > 0 && offset < payload_len {
        offset
    } else {
        payload_len
    };
    header_len - header_len % 8
}

/// SACK blocks of a packet (`data` is the whole packet); empty without `FLAG_SACK`.
pub(super) fn incoming_sacks(header: &Header, data: &[u8]) -> Vec<SackRange> {
    if !header.has_flag(FLAG_SACK) {
        return Vec::new();
    }
    let payload = data.get(HEADER_SIZE..).unwrap_or_default();
    decode_sack(&payload[..sack_block_bytes(header.data_offset, payload.len())]).unwrap_or_default()
}

impl StreamInner {
    /// D6: an invalid block is skipped; call after `on_cumulative_ack`.
    pub(super) fn on_sack(&mut self, ranges: &[SackRange]) -> AckedRateInfo {
        let now = Instant::now();
        let (acked, seq) = (self.remote_acked, self.next_send_seq());
        let mut out = Vec::new();
        for r in ranges
            .iter()
            .filter(|r| sack_block_valid(acked, seq, r.start, r.end))
        {
            // Sequence numbers only, no SACK: the block list is computed after the previous one is processed.
            let fresh = self.outgoing.unsacked_in(Seq(r.start), Seq(r.end));
            out.extend(fresh.into_iter().filter_map(|s| self.mark_sacked(s.0, now)));
        }
        out
    }

    fn mark_sacked(&mut self, seq: u32, now: Instant) -> Option<(u32, PacketRateInfo)> {
        self.zp.sack_lookups += 1;
        let pkt = self.outgoing.mark_sacked(Seq(seq), now)?;
        let (info, sent, resent) = (pkt.rate_info, pkt.time_sent, pkt.retransmit_count > 0);
        if !resent {
            // C `ack_packet`: an RTT sample from SACK too, if the packet was not retransmitted.
            self.sample_rtt(now.saturating_duration_since(sent).as_millis() as u32);
        }
        info.map(|info| (seq, info))
    }
}

#[cfg(test)]
#[path = "sack_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "sack_cost_tests.rs"]
mod cost_tests;
