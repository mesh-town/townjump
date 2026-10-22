// JS: dht-rpc/lib/io.js Request.destroy
//! `impl Io`: dropping in-flight requests of one owner (a finished query, a destroyed session).
//! Neighbours: `requests` creates them with an owner; `event_loop` times out the rest.

use super::state::Io;
use super::types::Owner;

impl Io {
    /// Drops every in-flight and queued request of `owner`; returns their tids.
    ///
    /// A dropped request gives no `RetryEvent`, no `TimeoutEvent`, and a late reply to its tid
    /// is ignored in `process_datagram`. The congestion window is released only for requests
    /// that were sent at least once (an unsent one never took a slot), as for a reply.
    pub(crate) fn cancel_owner(&mut self, owner: Owner) -> Vec<u16> {
        let mut tids = Vec::new();
        let mut i = 0;
        while i < self.inflight.len() {
            if self.inflight[i].owner != Some(owner) {
                i += 1;
                continue;
            }
            let entry = self.inflight.swap_remove(i);
            if entry.sent >= 1 {
                self.congestion.recv();
            }
            self.stats.active = self.stats.active.saturating_sub(1);
            self.stats.cancelled += 1;
            tids.push(entry.tid);
        }
        if !tids.is_empty() {
            self.pending.retain(|p| !tids.contains(&p.tid));
            tracing::debug!(?owner, ?tids, "cancel_owner");
        }
        tids
    }

    /// Tids of the in-flight requests of `owner`, ascending.
    #[cfg(test)]
    pub(crate) fn inflight_of(&self, owner: Owner) -> Vec<u16> {
        let mut tids: Vec<u16> = self
            .inflight
            .iter()
            .filter(|e| e.owner == Some(owner))
            .map(|e| e.tid)
            .collect();
        tids.sort_unstable();
        tids
    }
}
