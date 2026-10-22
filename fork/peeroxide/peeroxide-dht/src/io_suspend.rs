// JS: dht-rpc/lib/io.js (suspend, resume, onmessage `suspended` guard)
//! Freezing the IO layer: sockets stay bound (same port after resume), but nothing
//! is accepted or sent and every request in flight is forgotten.

use super::Io;

impl Io {
    /// True when no traffic may pass: destroying or suspended.
    pub(super) fn closed(&self) -> bool {
        self.destroying || self.suspended
    }

    /// Whether [`Io::suspend`] is in force.
    pub fn is_suspended(&self) -> bool {
        self.suspended
    }

    /// Stop accepting datagrams and forget requests in flight (JS `io.suspend` → `_clear(true)`).
    /// The caller fails whoever waited for them.
    pub fn suspend(&mut self) {
        self.suspended = true;
        self.inflight.clear();
        self.pending.clear();
        self.stats.active = 0;
        self.congestion.clear();
    }

    /// Accept and send again (JS `io.resume`; the sockets were never closed).
    pub fn resume(&mut self) {
        self.suspended = false;
    }
}
