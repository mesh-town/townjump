// JS: libudx/src/udx.c (writes_queued_bytes)
//! The write queue `send_queue` with the sum of packet lengths (C `writes_queued_bytes`): the length changes only here.
//! Related — `send_batch.rs` (who dequeues), `outgoing.rs` (`queue_for_send`, who enqueues).

use super::*;
use std::ops::Deref;

/// Packets waiting for the window and pacing. Read as a `VecDeque` (via `Deref`); change only through the methods below.
#[derive(Default)]
pub(super) struct SendQueue {
    queue: VecDeque<QueuedPacket>,
    /// Sum of `packet.len()` over the queue — the lengths the packets were queued with (before MTU probe padding).
    bytes: usize,
}

impl Deref for SendQueue {
    type Target = VecDeque<QueuedPacket>;

    fn deref(&self) -> &Self::Target {
        &self.queue
    }
}

impl SendQueue {
    /// Bytes in the queue, with headers (part of the `check_app_limited` input; the other part is `SentMap::unsacked_bytes`).
    pub(super) fn bytes(&self) -> usize {
        self.bytes
    }

    pub(super) fn push_back(&mut self, queued: QueuedPacket) {
        self.bytes += queued.packet.len();
        self.queue.push_back(queued);
    }

    /// Removes the head and subtracts its length before any packet edits after removal.
    pub(super) fn pop_front(&mut self) -> Option<QueuedPacket> {
        let queued = self.queue.pop_front()?;
        self.bytes -= queued.packet.len();
        Some(queued)
    }

    /// Redirects packets with address `from` (the `0.0.0.0:0` stub before the firewall hook) to `to`; lengths do not change.
    pub(super) fn readdress(&mut self, from: SocketAddr, to: SocketAddr) {
        for queued in self.queue.iter_mut().filter(|q| q.remote_addr == from) {
            queued.remote_addr = to;
        }
    }

    /// Checks the sum against a queue walk; in release the walk is not computed.
    pub(super) fn debug_check(&self) {
        debug_assert_eq!(
            self.bytes,
            self.queue.iter().map(|q| q.packet.len()).sum::<usize>(),
            "send_queue bytes diverged from the queue"
        );
    }
}
