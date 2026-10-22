//! End of stream (D8): the sequence number of our END, its confirmation by the peer and the reply ACK.
// JS: libudx/src/udx.c:process_packet (`ended` branch)
use super::wire::send_ack;
use super::*;

impl StreamInner {
    /// Reserves a number for our END; `prepare_end` and `queue_fin` do not diverge in bookkeeping.
    pub(crate) fn reserve_end_seq(&mut self) -> u32 {
        let seq = self.next_seq;
        self.next_seq = seq.wrapping_add(1);
        self.end_seq = Some(seq);
        seq
    }

    /// `true` exactly once: `remote_acked` has just stepped past our END's number.
    pub(super) fn take_end_acked(&mut self) -> bool {
        let Some(end_seq) = self.end_seq else {
            return false;
        };
        if self.end_acked || !seq_after(self.remote_acked, end_seq) {
            return false;
        }
        self.end_acked = true;
        true
    }

    /// Reply ACK to a confirmed END (JS: "final state packet").
    /// No reply: the peer has already finished (JS closes the stream), or the packet is DATA/END (the ACK for those already went out).
    pub(super) fn ack_confirmed_end(&self, type_flags: u8) {
        if self.remote_ended || type_flags & (FLAG_DATA | FLAG_END) != 0 {
            return;
        }
        send_ack(self);
    }
}

#[cfg(test)]
#[path = "end_tests.rs"]
mod tests;
