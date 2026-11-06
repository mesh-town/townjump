//! Read-path counters of the receiving side: what the stream handed to the reader's channel.

use std::time::Instant;

/// Bytes delivered into the read channel and when the last delivery happened.
#[derive(Default)]
pub(crate) struct ReadStats {
    pub(crate) delivered: u64,
    pub(crate) last_delivery: Option<Instant>,
}

impl ReadStats {
    pub(crate) fn record(&mut self, bytes: u64) {
        self.delivered += bytes;
        self.last_delivery = Some(Instant::now());
    }
}
