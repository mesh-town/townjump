//! Choice of SACK ranges over 50: the most recently changed ranges, in seq order (`sack_select.rs`).
use super::*;
use crate::native::header::{FLAG_DATA, SackRange};
use crate::native::stream::test_support::stream_inner;
use std::collections::HashSet;

const MAX: u32 = u32::MAX;

/// Hole `i` is the single packet `first + 2 * i`; packets arrive through every other seq.
fn hole(first: u32, i: u32) -> u32 {
    first.wrapping_add(2 * i)
}

fn arrive(inner: &mut StreamInner, seq: u32) {
    inner.receive_data(seq, vec![1], FLAG_DATA);
}

/// A stream waiting for `first - 2`, with `count` single-packet holes starting at `first`.
fn with_holes(first: u32, count: u32) -> StreamInner {
    let mut inner = stream_inner();
    inner.next_remote_seq = first.wrapping_sub(2);
    for i in 0..count {
        arrive(&mut inner, hole(first, i));
    }
    inner
}

fn starts(ranges: &[SackRange]) -> Vec<u32> {
    ranges.iter().map(|r| r.start).collect()
}

fn single(seq: u32) -> SackRange {
    SackRange {
        start: seq,
        end: seq.wrapping_add(1),
    }
}

fn ascending(ranges: &[SackRange]) -> bool {
    ranges
        .windows(2)
        .all(|w| crate::native::stream::seq::seq_diff(w[1].start, w[0].start) > 0)
}

#[test]
fn ten_holes_come_back_whole_and_in_seq_order() {
    let inner = with_holes(10, 10);
    let expected: Vec<SackRange> = (0..10).map(|i| single(hole(10, i))).collect();
    assert_eq!(inner.sack_ranges(), expected);
}

#[test]
fn sixty_holes_return_fifty_ranges_in_seq_order() {
    let ranges = with_holes(10, 60).sack_ranges();
    assert_eq!(ranges.len(), 50);
    assert!(ascending(&ranges), "{:?}", starts(&ranges));
}

#[test]
fn sixty_holes_keep_the_range_of_the_newest_arrival() {
    let mut inner = with_holes(10, 60);
    let top = hole(10, 60);
    arrive(&mut inner, top);
    let ranges = inner.sack_ranges();
    assert_eq!(ranges.len(), 50);
    assert_eq!(ranges.last(), Some(&single(top)));
}

#[test]
fn sixty_holes_keep_the_range_of_a_fresh_low_arrival() {
    let mut inner = with_holes(10, 60);
    // a repeat into the third hole from the bottom joins it to the hole above: [14, 17)
    arrive(&mut inner, hole(10, 2) + 1);
    let ranges = inner.sack_ranges();
    let merged = SackRange {
        start: hole(10, 2),
        end: hole(10, 3) + 1,
    };
    assert!(ranges.contains(&merged), "{:?}", starts(&ranges));
}

#[test]
fn sixty_holes_drop_the_ten_stalest_ranges() {
    let ranges = with_holes(10, 60).sack_ranges();
    let expected: Vec<u32> = (10..60).map(|i| hole(10, i)).collect();
    assert_eq!(starts(&ranges), expected);
}

#[test]
fn a_merged_range_takes_the_newer_stamp() {
    // the oldest range [2, 3), then 59 holes, then a fresh [4, 5), then 3 joins the old one to the fresh one
    let mut inner = stream_inner();
    arrive(&mut inner, 2);
    for k in 0..59 {
        arrive(&mut inner, 10 + 2 * k);
    }
    arrive(&mut inner, 4);
    arrive(&mut inner, 3);
    let ranges = inner.sack_ranges();
    let merged = SackRange { start: 2, end: 5 };
    assert_eq!(ranges.len(), 50);
    assert_eq!(ranges.first(), Some(&merged));
}

#[test]
fn every_arrival_is_announced_over_two_hundred_holes() {
    let mut inner = stream_inner();
    inner.next_remote_seq = 0;
    let mut announced: HashSet<u32> = HashSet::new();
    let mut received: Vec<u32> = Vec::new();
    for i in 0..200 {
        let seq = hole(2, i);
        arrive(&mut inner, seq);
        received.push(seq);
        for r in inner.sack_ranges() {
            announced.extend(r.start..r.end);
        }
    }
    let missing: Vec<u32> = received
        .into_iter()
        .filter(|s| !announced.contains(s))
        .collect();
    assert!(missing.is_empty(), "never announced: {missing:?}");
}

#[test]
fn stamps_survive_the_seq_wrap() {
    // 60 holes starting before the wrap and ending after it
    let first = MAX - 50;
    let mut inner = with_holes(first, 60);
    let top = hole(first, 60);
    arrive(&mut inner, top);
    let ranges = inner.sack_ranges();
    assert_eq!(ranges.len(), 50);
    assert!(ascending(&ranges), "{:?}", starts(&ranges));
    assert_eq!(ranges.last(), Some(&single(top)));
}
