//! Incoming ACK: cumulative ack, SACK, losses, fast retransmit and timer re-arm.
// JS: libudx/src/udx.c:process_packet (ack), udx_arm_rto_or_tlp
use super::timers::single_rto;
use super::*;
use std::time::Duration;

/// Result of ACK parsing: what to report, which timers to re-arm (`None` — leave alone).
pub(super) struct AckOutcome {
    pub(super) completed: WriteSenders,
    pub(super) rto_timer: Option<Duration>,
    pub(super) zwp_timer: Option<Duration>,
    /// RACK REO timer deadline; None — do not arm.
    pub(super) reo_timer: Option<Duration>,
    pub(super) kick_pacing: bool,
    /// This ACK was the first to confirm our END: a reply ACK is needed (`ack_confirmed_end`).
    pub(super) end_acked: bool,
    /// Adapter writer whose write headroom this ACK opened: wake it after the lock is released.
    pub(super) writer: Option<std::task::Waker>,
}

impl StreamInner {
    /// Parses a whole ACK packet; called under the lock from `process_incoming`.
    pub(super) fn on_ack_packet(
        &mut self,
        ack: u32,
        recv_window: u32,
        sacks: &[SackRange],
    ) -> AckOutcome {
        self.zp.proc_n += 1;
        let outcome = self.ack_outcome(ack, recv_window, sacks);
        self.settle_change();
        if diag::mode().z {
            self.diag_ack(outcome.kick_pacing);
        }
        outcome
    }

    /// Z: ACK side into the `ZS` roll-up; the writer is already taken by now if room opened.
    fn diag_ack(&self, kick: bool) {
        diag::ack(&diag::AckProbe {
            kick,
            open: self.congestion.can_send(self.inflight(), self.send_rwnd),
            parked: self.writer_waker.is_some(),
            room0: self.write_room() == 0,
            unsacked: self.outgoing.unsacked_bytes(),
            q_len: self.send_queue.len(),
            cwnd: self.congestion.cwnd,
            lost: self.outgoing.lost(),
        });
    }

    fn ack_outcome(&mut self, ack: u32, recv_window: u32, sacks: &[SackRange]) -> AckOutcome {
        self.send_rwnd = recv_window;
        let zwp_stop = self.zwp_stop_on_window();
        if self.window_only(ack, sacks) {
            return AckOutcome {
                completed: Vec::new(),
                rto_timer: None,
                zwp_timer: zwp_stop,
                reo_timer: None,
                kick_pacing: self.has_pending_send(),
                end_acked: false,
                writer: self.take_writer_if_room(),
            };
        }
        let before = self.remote_acked;
        let (completed, delivered, reo_timer) = self.apply_acks(ack, sacks);
        let advanced = self.remote_acked != before;
        AckOutcome {
            completed,
            rto_timer: self.rto_after_ack(advanced || delivered),
            zwp_timer: self.zwp_arm_after_ack(advanced).or(zwp_stop),
            reo_timer,
            kick_pacing: self.has_pending_send(),
            end_acked: self.take_end_acked(),
            writer: self.take_writer_if_room(),
        }
    }

    /// A packet without ACK and SACK, window only. `ack` 0 means "empty" until it confirms wraparound past 2^32.
    fn window_only(&self, ack: u32, sacks: &[SackRange]) -> bool {
        ack == 0 && sacks.is_empty() && !seq_after(0, self.remote_acked)
    }

    /// Result: completed writes, "something was delivered" (by cumulative ACK or SACK) and the REO deadline.
    pub(super) fn apply_acks(
        &mut self,
        ack: u32,
        sacks: &[SackRange],
    ) -> (WriteSenders, bool, Option<Duration>) {
        let dm = diag::mode();
        let zx_before =
            dm.z.then_some((self.congestion.ca_state, self.congestion.cwnd));
        let (completed, mut rate) = self.on_cumulative_ack(ack); // filters out an ack that does not move remote_acked itself
        if dm.f1 {
            // as C udx.c:1681: exit on cumulative ack before SACK parsing and before rack_detect_loss
            self.congestion.sack_exit = false;
            // an ack that did not move remote_acked (empty 0 or old) does not count as a cumulative ack
            if self.remote_acked == ack {
                self.f1_exit_by_cum_ack(ack);
            }
        }
        rate.extend(self.on_sack(sacks)); // after on_cumulative_ack: D6 sees the new remote_acked
        let delivered = !rate.is_empty(); // C `delivered > 0`: RTO re-arm (udx.c:1751-1765)
        let (lost, reo_timer) = self.detect_losses_at(Instant::now());
        self.zp.marks_ack += u64::from(lost);
        if !rate.is_empty() || lost > 0 {
            let inflight = self.inflight();
            // C: rs->losses is always 0 (udx.c:1747, before rack_detect_loss 1769)
            self.congestion.set_rtt_min(self.outgoing.rack().rtt_min_ms);
            self.congestion.on_ack(&rate, 0, inflight, Instant::now());
        }
        if let Some((state, cwnd_before)) = zx_before {
            self.diag_exit(state, cwnd_before, ack, &rate);
        }
        self.debug_check_counters();
        (completed, delivered, reo_timer)
    }

    /// F1 (C `udx.c:1681`): exit from recovery/loss by the header's cumulative `ack`, strictly `ack > high_seq`.
    pub(super) fn f1_exit_by_cum_ack(&mut self, ack: u32) {
        if self.congestion.ca_state >= CaState::Recovery && seq_after(ack, self.congestion.high_seq)
        {
            self.congestion.ca_state = CaState::Open;
        }
    }

    /// Z: `ZX` line if after ACK parsing the state is Open and before it was Recovery or Loss.
    fn diag_exit(
        &self,
        before: CaState,
        cwnd_before: u32,
        ack: u32,
        rate: &[(u32, crate::native::congestion::rate::PacketRateInfo)],
    ) {
        if before < CaState::Recovery || self.congestion.ca_state != CaState::Open {
            return;
        }
        let max_acked = rate
            .iter()
            .map(|(seq, _)| *seq)
            .reduce(|a, b| if seq_after(b, a) { b } else { a })
            .unwrap_or(0);
        let high_seq = self.congestion.high_seq;
        diag::exit(&diag::Exit {
            state_before: before as u8,
            cum: ack,
            max_acked,
            high_seq,
            cum_gt_high: seq_after(ack, high_seq),
            cwnd_before,
            cwnd_after: self.congestion.cwnd,
            inflight: self.inflight(),
            lost: self.outgoing.lost(),
            sacked: self.outgoing.sacked(),
        });
    }

    /// RACK REO timer (C `udx_rack_reo_timeout`, 1135-1145): time-based marking without an ACK; REO is not re-armed.
    pub(super) fn on_reo_fire(&mut self) -> bool {
        self.zp.marks_timer += u64::from(self.detect_losses_at(Instant::now()).0);
        self.outgoing.lost() > 0
    }

    fn rto_after_ack(&self, progress: bool) -> Option<Duration> {
        if self.outgoing.is_empty() {
            return Some(FAR_FUTURE);
        }
        progress.then(|| single_rto(self.rto))
    }
}

#[cfg(test)]
#[path = "acks_tests.rs"]
mod tests;
