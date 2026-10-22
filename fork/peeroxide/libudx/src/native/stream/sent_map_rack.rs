//! RACK walk of the outgoing map in send order (C inflight_queue, udx.c:1043-1120) — the only production path;
//! the walk by numbers remains the tests' reference (twins in `sent_map_rack_twin.rs`).

use super::*;
use std::collections::BTreeSet;

/// Send-order index: key (last send time, number) for entries without SACK and loss.
/// `Default` is enabled and empty; disabled (the by-number reference) only in tests: the hooks do nothing.
pub(super) struct SendOrder(Option<BTreeSet<(Instant, u32)>>);

impl Default for SendOrder {
    fn default() -> Self {
        Self(Some(BTreeSet::new()))
    }
}

impl SendOrder {
    /// Tests: a disabled index.
    #[cfg(test)]
    pub(super) fn disabled() -> Self {
        Self(None)
    }

    pub(in crate::native::stream) fn is_enabled(&self) -> bool {
        self.0.is_some()
    }

    /// An entry enters the index (insertion at the tail, C udx.c:680).
    pub(super) fn add(&mut self, seq: Seq, time_sent: Instant) {
        if let Some(set) = &mut self.0 {
            let fresh = set.insert((time_sent, seq.0));
            debug_assert!(fresh, "key {seq:?} is already in the send-order index");
        }
    }

    /// An entry leaves the index (SACK, loss, ACK removal).
    pub(super) fn remove(&mut self, seq: Seq, time_sent: Instant) {
        if let Some(set) = &mut self.0 {
            let had = set.remove(&(time_sent, seq.0));
            debug_assert!(had, "key {seq:?} is not in the send-order index");
        }
    }

    /// Numbers with `time < rack_sent` in ascending time.
    fn sent_before(&self, rack_sent: Instant) -> impl Iterator<Item = Seq> + '_ {
        let range = self.0.iter().flat_map(move |s| s.range(..(rack_sent, 0)));
        range.map(|&(_, seq)| Seq(seq))
    }

    /// Snapshot of keys in ascending order; `None` — disabled.
    pub(super) fn keys(&self) -> Option<Vec<(Instant, u32)>> {
        self.0.as_ref().map(|s| s.iter().copied().collect())
    }

    pub(super) fn len(&self) -> usize {
        self.0.as_ref().map_or(0, BTreeSet::len)
    }
}

impl SentMap {
    /// RACK by time (C `rack_detect_loss`, udx.c:1044-1120): a packet without SACK and loss, sent before
    /// `rack.sent`, is lost when `time_sent + rack.rtt + reo_wnd <= now`, otherwise its remainder goes to the timer.
    /// Walk by numbers up to the first original packet not earlier than `rack.sent` (C walks in send order up to `break`):
    /// original packets go out in ascending number, a retransmission later than the original. Result — numbers and the largest remainder.
    /// There is one path — by the send-order index; by numbers (`mark_rack_lost_scan`) is the tests' reference when the index is disabled.
    pub(in crate::native::stream) fn mark_rack_lost(
        &mut self,
        reo_wnd: Duration,
        now: Instant,
    ) -> (Vec<Seq>, Option<Duration>) {
        #[cfg(test)]
        if !self.order.is_enabled() {
            return self.mark_rack_lost_scan(reo_wnd, now);
        }
        self.mark_rack_lost_indexed(reo_wnd, now)
    }

    #[cfg(test)]
    fn mark_rack_lost_scan(
        &mut self,
        reo_wnd: Duration,
        now: Instant,
    ) -> (Vec<Seq>, Option<Duration>) {
        debug_assert!(!self.order.is_enabled());
        let Some(rack_sent) = self.rack.sent else {
            return (Vec::new(), None);
        };
        let wait = self.rack.rtt + reo_wnd;
        let mut timeout = None;
        let mut visited = 0u32;
        let lost: Vec<Seq> = self
            .map
            .iter_mut()
            .take_while(|(_, p)| {
                visited += 1;
                p.retransmit_count > 0 || p.time_sent < rack_sent
            })
            .filter(|(_, p)| !p.sacked && !p.lost && p.time_sent < rack_sent)
            .filter_map(|(&seq, p)| expire(p, wait, now, &mut timeout).then_some(seq))
            .collect();
        self.rack_visited = visited;
        lost.iter().for_each(|&seq| self.lost_seqs.add(seq));
        self.lost += lost.len() as u32;
        (lost, timeout)
    }

    fn mark_rack_lost_indexed(
        &mut self,
        reo_wnd: Duration,
        now: Instant,
    ) -> (Vec<Seq>, Option<Duration>) {
        let Some(rack_sent) = self.rack.sent else {
            return (Vec::new(), None);
        };
        let wait = self.rack.rtt + reo_wnd;
        #[cfg(debug_assertions)]
        let preview = self.rack_scan_preview(wait, rack_sent, now);
        let mut timeout = None;
        let mut visited = 0u32;
        let mut lost = Vec::new();
        for seq in self.order.sent_before(rack_sent) {
            visited += 1;
            let pkt = self.map.get_mut(&seq);
            let pkt = pkt.expect("the send-order index points at a map entry");
            debug_assert!(
                !pkt.sacked && !pkt.lost,
                "entry {seq:?} in the index with the flag"
            );
            if expire(pkt, wait, now, &mut timeout) {
                lost.push(seq);
            }
        }
        for seq in &lost {
            self.order.remove(*seq, self.map[seq].time_sent);
        }
        lost.sort_unstable();
        self.rack_visited = visited;
        lost.iter().for_each(|&seq| self.lost_seqs.add(seq));
        self.lost += lost.len() as u32;
        // Equality of the paths is held by the twins; in a race of sync writers the index marks more (as C), hence ⊆.
        #[cfg(debug_assertions)]
        debug_assert!(
            preview.0.iter().all(|s| lost.contains(s)) && preview.1 <= timeout,
            "RACK by index lost the marks of the old walk"
        );
        (lost, timeout)
    }

    /// Result of the by-number rule without changing the map (read only) — a debug cross-check: losses ⊆ index ones.
    #[cfg(debug_assertions)]
    fn rack_scan_preview(
        &self,
        wait: Duration,
        rack_sent: Instant,
        now: Instant,
    ) -> (Vec<Seq>, Option<Duration>) {
        let mut timeout = None;
        let walk = self
            .map
            .iter()
            .take_while(|(_, p)| p.retransmit_count > 0 || p.time_sent < rack_sent);
        let mut lost = Vec::new();
        for (&seq, p) in walk.filter(|(_, p)| !p.sacked && !p.lost && p.time_sent < rack_sent) {
            let remaining = (p.time_sent + wait).saturating_duration_since(now);
            if remaining.is_zero() {
                lost.push(seq);
            } else {
                timeout = timeout.max(Some(remaining));
            }
        }
        (lost, timeout)
    }

    /// Cross-check of the index against the map: keys of entries without SACK and loss and their count.
    pub(super) fn debug_check_order(&self) {
        if self.order.is_enabled() {
            debug_assert_eq!(
                self.order.keys().unwrap_or_default(),
                self.recount_order(),
                "send-order index diverged from the map"
            );
            debug_assert_eq!(self.order.len(), self.inflight() as usize);
        }
    }

    fn recount_order(&self) -> Vec<(Instant, u32)> {
        let live = self.map.iter().filter(|(_, p)| !p.sacked && !p.lost);
        let mut keys: Vec<_> = live.map(|(&seq, p)| (p.time_sent, seq.0)).collect();
        keys.sort_unstable();
        keys
    }
}

/// The packet's deadline has passed — sets `lost`; otherwise accumulates the largest remainder (C `remaining > timeout`).
fn expire(
    pkt: &mut SentPacket,
    wait: Duration,
    now: Instant,
    timeout: &mut Option<Duration>,
) -> bool {
    let remaining = (pkt.time_sent + wait).saturating_duration_since(now);
    if !remaining.is_zero() {
        *timeout = (*timeout).max(Some(remaining));
        return false;
    }
    pkt.lost = true;
    true
}

#[cfg(test)]
#[path = "sent_map_rack_twin.rs"]
mod twin;

#[cfg(test)]
#[path = "sent_map_rack_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "sent_map_rack_prop_tests.rs"]
mod prop_tests;
