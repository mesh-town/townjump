//! Teardown of a `Holepuncher`: explicit `destroy()` and the initiator's `Drop`.

use super::{HolepunchEvent, Holepuncher};
use std::sync::atomic::Ordering;

impl Holepuncher {
    pub fn destroy(&mut self) {
        if self.destroyed {
            return;
        }
        self.release();
        if !self.connected && !self.connected_flag.load(Ordering::Acquire) {
            let _ = self.event_tx.send(HolepunchEvent::Aborted);
        }
    }

    /// Frees every socket and aborts every task; no event is sent.
    fn release(&mut self) {
        self.destroyed = true;
        self.punching = false;
        self.sockets.clear();
        self.samples = None;
        self.nat.destroy();
        for h in self.recv_tasks.drain(..) {
            h.abort();
        }
    }
}

/// An initiator that leaves scope (Connected, early `?`, cancelled future) must not leak its
/// birthday sockets; the primary socket survives through the clone held by the connection.
impl Drop for Holepuncher {
    fn drop(&mut self) {
        if self.is_initiator && !self.destroyed {
            self.release();
        }
    }
}
