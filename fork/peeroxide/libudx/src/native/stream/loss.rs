// JS: libudx/src/udx.c
//! Reliability: RTT, cumulative ACK, loss marking (RACK, RTO); retransmission is in `retransmit.rs`; see also `outgoing.rs`, `receive.rs`.

use super::*;
use std::time::Duration;

impl StreamInner {
    // ── RTT estimation (Jacobson/Karels) ─────────────────────────────────────

    /// Update the RTT estimator with a new sample.
    ///
    /// Uses standard constants matching C libudx:
    ///   α = 1/8 (SRTT smoothing)
    ///   β = 1/4 (RTTVAR smoothing)
    ///   K = 4   (RTO multiplier for RTTVAR)
    ///   G = 1000ms (minimum RTO granularity)
    pub(super) fn update_rtt(&mut self, rtt_ms: u32) {
        let rtt = self.clamp_rtt(rtt_ms).clamp(1, MAX_RTT_MS);
        if self.srtt == 0 {
            // First sample
            self.srtt = rtt;
            self.rttvar = rtt / 2;
        } else {
            let delta = self.srtt.abs_diff(rtt);
            // RTTVAR <- (3/4) * RTTVAR + (1/4) * |SRTT - R|
            self.rttvar = (3 * self.rttvar + delta) / 4;
            // SRTT <- (7/8) * SRTT + (1/8) * R
            self.srtt = (7 * self.srtt + rtt) / 8;
        }
        // RTO <- SRTT + max(G, K * RTTVAR) where K=4, G=1000ms
        self.rto = (self.srtt + 4 * self.rttvar).clamp(1_000, MAX_RTO_MS);
    }

    /// Outlier as C `clamp_rtt` (`udx.c:1242-1255`): a sample above `srtt + 5*rttvar` and above 5000 ms
    /// is clamped to `srtt + 5*rttvar`; the first sample (`srtt == 0`) is left alone.
    fn clamp_rtt(&self, rtt_ms: u32) -> u32 {
        let limit = self.srtt.saturating_add(5 * self.rttvar);
        if self.srtt == 0 || rtt_ms <= limit || rtt_ms <= 5_000 {
            return rtt_ms;
        }
        limit.min(MAX_RTT_MS)
    }

    /// RTT sample: accounting in the ZP probe (if enabled) and the estimate update.
    pub(super) fn sample_rtt(&mut self, elapsed_ms: u32) {
        if diag::mode().z {
            self.zp.rtt.note_sample(elapsed_ms);
        }
        self.update_rtt(elapsed_ms);
    }

    // ── ACK processing ───────────────────────────────────────────────────────

    /// Process a cumulative ACK from the remote peer.
    ///
    /// Removes ACKed packets from the outgoing buffer, measures RTT from
    /// every removed packet without retransmits and without SACK (a SACKed packet
    /// waited for the hole: its `time_sent.elapsed()` is not an RTT sample; C `ack_packet`),
    /// resets the RTO timeout counter, and returns completed write senders plus
    /// rate info for congestion control.
    ///
    /// Congestion control is NOT called here — the caller must combine this
    /// rate info with SACK rate info and loss count, then call `congestion.on_ack`.
    pub(super) fn on_cumulative_ack(&mut self, ack: u32) -> (WriteSenders, AckedRateInfo) {
        if !seq_after(ack, self.remote_acked) {
            return (Vec::new(), Vec::new());
        }

        self.remote_acked = ack;
        if diag::mode().z {
            self.zp.rtt.note_cum();
        }

        let removed = self.outgoing.remove_before(Seq(ack), Instant::now());

        let mut acked_rate_info: Vec<(u32, super::congestion::rate::PacketRateInfo)> = Vec::new();
        for (&Seq(seq), pkt) in &removed {
            if pkt.retransmit_count == 0 && !pkt.sacked {
                self.sample_rtt(pkt.time_sent.elapsed().as_millis() as u32);
            }
            // A packet already delivered via SACK does not enter the rate input again (C `ack_packet`: the packet is removed at SACK).
            if let Some(info) = pkt.rate_info.filter(|_| !pkt.sacked) {
                acked_rate_info.push((seq, info));
            }
            if pkt.is_mtu_probe && self.mtu_state == MtuState::Search {
                self.on_mtu_probe_acked();
            }
        }

        // Successful ACK resets timeout counter
        self.rto_timeouts = 0;

        // Complete pending writes whose ack_threshold <= ack
        let mut completed = Vec::new();
        self.pending_writes.retain_mut(|pw| {
            if seq_diff(ack, pw.ack_threshold) >= 0 {
                completed.push(pw.tx.take());
                false
            } else {
                true
            }
        });
        (completed, acked_rate_info)
    }

    // ── Loss detection ───────────────────────────────────────────────────────

    /// C `rack_update_reo_wnd` (udx.c:545-558): before reordering is seen — 0 outside Open or with >= 3 SACKs; otherwise (and always once seen) `min(rtt_min/4, srtt)`.
    pub(super) fn reo_wnd(&self) -> Duration {
        if !self.outgoing.rack().reordering_seen
            && (self.congestion.ca_state != CaState::Open || self.outgoing.sacked() >= 3)
        {
            return Duration::ZERO;
        }
        let quarter = self.outgoing.rack().rtt_min_ms.map_or(0, |min| min / 4);
        Duration::from_millis(u64::from(quarter.min(self.srtt)))
    }

    /// RACK losses by time (C `rack_detect_loss`): a packet without SACK, sent before the latest
    /// delivered one, is lost when `rtt + reo_wnd` expires; `reo_wnd` is taken before marking (C 1045). A lost MTU
    /// probe counts in the result (as C `stream->lost++`) but does not lead to `Recovery`. Result — the number of new losses and the
    /// REO timer deadline from the largest remainder of the waiting ones (C 1123-1133).
    pub(super) fn detect_losses_at(&mut self, now: Instant) -> (u32, Option<Duration>) {
        let reo_wnd = self.reo_wnd();
        let (lost_seqs, timeout) = self.outgoing.mark_rack_lost(reo_wnd, now);
        let diag_z = diag::mode().z;
        if diag_z {
            diag::rack(self.outgoing.rack_visited);
            self.zp.rack_visits += u64::from(self.outgoing.rack_visited);
        }

        let was_open = self.congestion.ca_state == CaState::Open;
        self.count_losses(&lost_seqs);

        // Record the recovery boundary: when all packets up to high_seq
        // are ACK'd, we can exit Recovery → Open.
        if was_open && self.congestion.ca_state == CaState::Recovery {
            self.zp.marks_enter += lost_seqs.len() as u64;
            if let Some(&Seq(max_seq)) = self.outgoing.keys().last() {
                self.congestion.high_seq = max_seq;
            }
            if diag_z {
                diag::enter(
                    self.congestion.high_seq,
                    self.congestion.cwnd,
                    self.inflight(),
                    lost_seqs.len() as u32,
                    self.remote_acked,
                );
            }
        }

        (lost_seqs.len() as u32, timeout)
    }

    /// Z: snapshot for the `ZP` line; `cwnd_cmp` and `cwnd_lim` are both parts of the `can_send` comparison.
    pub(super) fn zp_view(&self) -> diag::ZpView {
        let rack = self.outgoing.rack();
        diag::ZpView {
            remote_acked: self.remote_acked,
            seq: self.next_seq,
            map_len: self.outgoing.len(),
            map_min: self.outgoing.keys().next().map_or(0, |s| s.0),
            sacked: self.outgoing.sacked(),
            lost: self.outgoing.lost(),
            inflight: self.inflight(),
            cwnd_cmp: self.outgoing.inflight(),
            cwnd_lim: self.congestion.send_limit(self.send_rwnd),
            rack_rtt_ms: rack.rtt.as_millis() as u64,
            reo_wnd_ms: self.reo_wnd().as_millis() as u64,
            min_rtt_ms: rack.rtt_min_ms,
        }
    }

    /// Tests: mark now, no timer deadline.
    #[cfg(test)]
    pub(super) fn detect_losses(&mut self) -> u32 {
        self.detect_losses_at(Instant::now()).0
    }

    /// The number is a sent MTU probe: the flag is set before retransmission, padding is removed by `resend_*` (`sent_map.rs`), later than `detect_losses`.
    fn is_probe(&self, seq: Seq) -> bool {
        self.outgoing.get(&seq).is_some_and(|pkt| pkt.is_mtu_probe)
    }

    // ── RTO marking ──────────────────────────────────────────────────────────

    /// Losses into the congestion counter; an MTU probe is not congestion (C `resending > mtu_probes_lost`, RFC 8899).
    fn count_losses(&mut self, seqs: &[Seq]) {
        let real: Vec<u32> = seqs
            .iter()
            .filter(|&&s| !self.is_probe(s))
            .map(|s| s.0)
            .collect();
        real.into_iter()
            .for_each(|seq| self.congestion.on_packet_lost(seq));
    }

    /// RTO marking (C 1195-1231): `reo_wnd` is 0 in Loss unless reordering was seen, overdue ones are `lost`, window — in flight + 1.
    pub(super) fn mark_rto_losses(&mut self, now: Instant) {
        let reo_wnd = self.reo_wnd();
        let lost = self.outgoing.mark_rto_lost(reo_wnd, now);
        self.count_losses(&lost);
        self.zp.marks_rto += lost.len() as u64;
        let inflight = self.inflight();
        self.congestion.cut_cwnd_to_flight(inflight);
    }

    /// A retransmitted MTU probe did not arrive: counts a failure, after `MTU_MAX_PROBES` the search is over.
    pub(super) fn on_probe_resent(&mut self, path: &str) {
        self.mtu_probe_count += 1;
        tracing::debug!(
            failures = self.mtu_probe_count,
            max = MTU_MAX_PROBES,
            "mtu: probe failed ({path})"
        );
        if self.mtu_probe_count >= MTU_MAX_PROBES {
            self.mtu_state = MtuState::SearchComplete;
            self.mtu_probe_wanted = false;
        } else {
            self.mtu_probe_wanted = true;
        }
    }
}

#[cfg(test)]
#[path = "zp_marks_tests.rs"]
mod zp_marks_tests;

#[cfg(test)]
#[path = "rtt_sample_tests.rs"]
mod rtt_sample_tests;
