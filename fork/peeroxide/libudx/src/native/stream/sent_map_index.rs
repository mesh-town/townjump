//! Index of `outgoing` map numbers (`Seq` order, with wraparound): entries without SACK and entries with loss.
//! Needed so that a SACK block and `first_lost` scale with the number of marked entries, not the map size (C uses lists
//! `inflight_queue` / `retransmit_queue`, udx.c).

use super::*;
use std::collections::BTreeSet;
use std::ops::Bound;

#[derive(Default)]
pub(super) struct SeqIndex(BTreeSet<Seq>);

impl SeqIndex {
    /// A number enters the index.
    pub(super) fn add(&mut self, seq: Seq) {
        let fresh = self.0.insert(seq);
        debug_assert!(fresh, "number {seq:?} is already in the index");
    }

    /// A number leaves the index.
    pub(super) fn remove(&mut self, seq: Seq) {
        let had = self.0.remove(&seq);
        debug_assert!(had, "number {seq:?} is not in the index");
    }

    /// Drops all numbers before `ack` (`Seq` order).
    pub(super) fn split_before(&mut self, ack: Seq) {
        let kept = self.0.split_off(&ack);
        self.0 = kept;
    }

    /// Numbers in `[start, end)` ascending; `start >= end` — empty.
    pub(super) fn range_vec(&self, start: Seq, end: Seq) -> Vec<Seq> {
        if start >= end {
            return Vec::new();
        }
        self.0.range(start..end).copied().collect()
    }

    /// The smallest number strictly after `after` (`None` — from the head).
    pub(super) fn first_after(&self, after: Option<Seq>) -> Option<Seq> {
        let from = after.map_or(Bound::Unbounded, Bound::Excluded);
        self.0.range((from, Bound::Unbounded)).next().copied()
    }

    pub(super) fn len(&self) -> usize {
        self.0.len()
    }

    /// Cross-check against a recount from the map; `expect` — the numbers that must be in the index.
    #[cfg(debug_assertions)]
    pub(super) fn debug_check(&self, expect: impl Iterator<Item = Seq>, what: &str) {
        let mut want: Vec<Seq> = expect.collect();
        want.sort_unstable();
        let have: Vec<Seq> = self.0.iter().copied().collect();
        debug_assert_eq!(have, want, "index {what} diverged from the map");
    }

    #[cfg(not(debug_assertions))]
    pub(super) fn debug_check(&self, _expect: impl Iterator<Item = Seq>, _what: &str) {}
}

#[cfg(test)]
#[path = "sent_map_index_tests.rs"]
mod tests;
