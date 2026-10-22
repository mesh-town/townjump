// JS: libudx/src/udx.c (UDX_HIGH_WATERMARK, send_window_in_bytes, on_bytes_acked)
//! Adapter write budget: write queue headroom and writer wakeup, as in C.

use super::*;
use std::task::Waker;

impl StreamInner {
    /// Payload bytes not yet sent or sent without ACK/SACK (C `writes_queued_bytes`).
    fn write_queued(&self) -> usize {
        let bytes = self.send_queue.bytes() + self.outgoing.unsacked_bytes();
        let packets =
            self.send_queue.len() + (self.outgoing.inflight() + self.outgoing.lost()) as usize;
        bytes.saturating_sub(HEADER_SIZE * packets)
    }

    /// `WRITE_HIGH_WATERMARK + min(cwnd·max_payload, send_rwnd)`; recomputed on every call.
    fn write_budget(&self) -> usize {
        let window = (self.congestion.cwnd as usize).saturating_mul(self.max_payload());
        let rwnd = self.send_rwnd as usize;
        WRITE_HIGH_WATERMARK.saturating_add(window.min(rwnd))
    }

    /// How many more bytes the adapter may accept; 0 means the writer waits for ACKs.
    pub(crate) fn write_room(&self) -> usize {
        self.write_budget().saturating_sub(self.write_queued())
    }

    /// Registers the writer for wakeup; the previous waker is replaced.
    pub(crate) fn park_writer(&mut self, waker: &Waker) {
        self.writer_waker = Some(waker.clone());
    }

    /// Hands out the waker only if headroom > 0 (otherwise it stays in place).
    pub(super) fn take_writer_if_room(&mut self) -> Option<Waker> {
        if self.writer_waker.is_none() || self.write_room() == 0 {
            if self.writer_waker.is_some() && diag::mode().z {
                diag::writer(false, self.write_room());
            }
            return None;
        }
        if diag::mode().z {
            diag::writer(true, self.write_room());
        }
        self.writer_waker.take()
    }
}

#[cfg(test)]
#[path = "write_budget_tests.rs"]
mod tests;
