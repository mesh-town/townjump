//! Which SACK ranges go into one ACK when the receiver holds more than `MAX_SACK_RANGES` of them.
//! The newest-changed ranges win (arrival stamps), so every arrival is announced once; the packet stays in seq order.
use std::collections::BTreeMap;

use super::consts::MAX_SACK_RANGES;
use super::seq::Seq;
use super::types::RecvdPacket;
use crate::native::header::SackRange;

/// A contiguous range of buffered seqs and the newest arrival stamp inside it.
struct Stamped {
    range: SackRange,
    newest: u64,
}

/// All ranges when there are at most `MAX_SACK_RANGES`; otherwise the `MAX_SACK_RANGES` with the largest stamps.
/// The result is in ascending seq order (wrap-aware, as the buffer keys are).
pub(super) fn select_ranges(buf: &BTreeMap<Seq, RecvdPacket>) -> Vec<SackRange> {
    let all = stamped_ranges(buf);
    if all.len() <= MAX_SACK_RANGES {
        return all.into_iter().map(|s| s.range).collect();
    }
    let floor = stamp_floor(&all);
    let kept = all.into_iter().filter(|s| s.newest >= floor);
    kept.map(|s| s.range).collect()
}

fn stamped_ranges(buf: &BTreeMap<Seq, RecvdPacket>) -> Vec<Stamped> {
    let mut out: Vec<Stamped> = Vec::new();
    for (&Seq(seq), packet) in buf {
        match out.last_mut() {
            Some(last) if last.range.end == seq => {
                last.range.end = seq.wrapping_add(1);
                last.newest = last.newest.max(packet.stamp);
            }
            _ => out.push(Stamped {
                range: SackRange {
                    start: seq,
                    end: seq.wrapping_add(1),
                },
                newest: packet.stamp,
            }),
        }
    }
    out
}

/// The `MAX_SACK_RANGES`-th largest stamp: ranges at or above it are kept (stamps are unique per packet).
fn stamp_floor(all: &[Stamped]) -> u64 {
    let mut stamps: Vec<u64> = all.iter().map(|s| s.newest).collect();
    stamps.sort_unstable_by(|a, b| b.cmp(a));
    stamps[MAX_SACK_RANGES - 1]
}
