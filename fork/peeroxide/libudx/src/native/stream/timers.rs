//! Stream timers: RTO (D7) and the zero-window probe (D4). The stream loop is stream.rs:process_incoming.
// JS: libudx/src/udx.c:udx_rto_timeout, udx_zwp_timeout, udx_stream_write
use super::wire::{heartbeat_packet, send_raw};
use super::*;
use std::time::Duration;

/// Interval "retransmit after rto×2": `rto` itself does not grow.
pub(super) fn double_rto(rto_ms: u32) -> Duration {
    Duration::from_millis(u64::from(rto_ms) * 2)
}

/// Interval "after rto".
pub(super) fn single_rto(rto_ms: u32) -> Duration {
    Duration::from_millis(u64::from(rto_ms))
}

impl StreamInner {
    /// Some(FAR_FUTURE) — nothing to retransmit; Some(rto×2) — marked; None — the 7th in a row, close.
    /// Retransmissions are sent by the sender on a kick (C `send_packets` 1234).
    pub(super) fn on_rto_fire(&mut self) -> Option<Duration> {
        if self.outgoing.is_empty() {
            return Some(FAR_FUTURE);
        }
        self.rto_fires = self.rto_fires.wrapping_add(1);
        self.rto_timeouts += 1;
        if self.rto_timeouts > MAX_RTO_TIMEOUTS {
            self.fail_writes_timed_out();
            return None;
        }
        self.enter_rto_loss();
        self.mark_rto_losses(Instant::now());
        Some(double_rto(self.rto)) // rto itself does not grow
    }

    fn enter_rto_loss(&mut self) {
        self.congestion.on_rto();
        let Some(&Seq(max_seq)) = self.outgoing.keys().last() else {
            return;
        };
        self.congestion.high_seq = max_seq;
    }

    fn fail_writes_timed_out(&mut self) {
        for mut pw in std::mem::take(&mut self.pending_writes) {
            let Some(tx) = pw.tx.take() else { continue };
            let _ = tx.send(Err(UdxError::Io(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "RTO timeout exceeded",
            ))));
        }
    }

    fn zwp_blocked(&self) -> bool {
        self.send_rwnd == 0 && self.outgoing.is_empty() && !self.send_queue.is_empty()
    }

    /// A write into an idle stream at window 0: probe at once, timer on rto.
    pub(super) fn zwp_start_on_write(&mut self) -> Option<Duration> {
        if self.zwp_active || !self.zwp_blocked() {
            return None;
        }
        self.zwp_active = true;
        self.send_probe();
        Some(single_rto(self.rto))
    }

    /// An ACK acked everything sent, window 0, data in the queue: timer on rto without a probe.
    pub(super) fn zwp_arm_after_ack(&mut self, advanced: bool) -> Option<Duration> {
        if !advanced || self.zwp_active || !self.zwp_blocked() {
            return None;
        }
        self.zwp_active = true;
        Some(single_rto(self.rto))
    }

    /// The window opened — ZWP is cleared.
    pub(super) fn zwp_stop_on_window(&mut self) -> Option<Duration> {
        if self.send_rwnd == 0 || !self.zwp_active {
            return None;
        }
        self.zwp_active = false;
        Some(FAR_FUTURE)
    }

    /// ZWP fire: probe and rto×2; the window is already open — turn off.
    pub(super) fn on_zwp_fire(&mut self) -> Duration {
        if !self.zwp_active || self.send_rwnd > 0 {
            self.zwp_active = false;
            return FAR_FUTURE;
        }
        self.send_probe();
        double_rto(self.rto)
    }

    /// HEARTBEAT 20 bytes: seq — the first unsent (not consumed), ack — next_remote_seq.
    fn send_probe(&self) {
        send_raw(
            self,
            heartbeat_packet(self.remote_id, self.next_send_seq(), self.next_remote_seq),
        );
    }
}

#[cfg(test)]
#[path = "timers_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "rto_loss_tests.rs"]
mod rto_loss_tests;
