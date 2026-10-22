//! Packet number modulo 2^32: signed difference, `Seq` as the key of the `outgoing` and `recv_buf` maps.
// JS: libudx/src/udx.c:seq_compare, seq_diff
use std::cmp::Ordering;

/// Signed difference of numbers modulo 2^32.
pub(super) fn seq_diff(a: u32, b: u32) -> i32 {
    a.wrapping_sub(b) as i32
}

/// `a` is strictly later than `b` (accounting for wraparound past 2^32).
pub(super) fn seq_after(a: u32, b: u32) -> bool {
    seq_diff(a, b) > 0
}

/// Packet number ordered by signed difference: `MAX < 0 < 1`.
///
/// The order is consistent only while numbers lie within a window ≪ 2^31 — true for packets in flight.
/// So `BTreeMap<Seq, _>` does not break on wraparound, unlike `BTreeMap<u32, _>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Seq(pub(super) u32);

impl Ord for Seq {
    fn cmp(&self, other: &Self) -> Ordering {
        seq_diff(self.0, other.0).cmp(&0)
    }
}

impl PartialOrd for Seq {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
