//! Each packet enters the controller input (`congestion.on_ack`) once. C `ack_packet` (udx.c:1259) removes a packet from
//! `outgoing` on SACK, and a cumulative ACK arriving later returns 0 — `delivered` does not grow twice.
//! Here a SACK only marks the packet and the cumulative ACK removes it later, so the removal must not feed `delivered` a second time (that would overstate the delivery count and rate).
use super::test_support::{stream_inner, with_sent};
use super::*;

fn seqs(rate: &[(u32, crate::native::congestion::rate::PacketRateInfo)]) -> Vec<u32> {
    rate.iter().map(|(seq, _)| *seq).collect()
}

/// Five packets 1..=5, 2 and 3 acked by SACK, then cumulative ACK 5 removes 1..=4.
/// Four distinct packets are delivered: 2, 3 (SACK) and 1, 4 (cumulatively).
#[test]
fn a_packet_acked_by_sack_is_not_counted_again_by_the_cumulative_ack() {
    let mut inner = stream_inner();
    with_sent(&mut inner, 1, 5);
    let by_sack = inner.on_sack(&[SackRange { start: 2, end: 4 }]);
    let (_, by_cumulative) = inner.on_cumulative_ack(5);
    let mut all = seqs(&by_sack);
    all.extend(seqs(&by_cumulative));
    all.sort_unstable();
    assert_eq!(all, vec![1, 2, 3, 4], "each packet exactly once");
}

/// A cumulative ACK without SACKs still returns all removed packets.
#[test]
fn cumulative_ack_without_sack_returns_every_removed_packet() {
    let mut inner = stream_inner();
    with_sent(&mut inner, 1, 5);
    let (_, by_cumulative) = inner.on_cumulative_ack(4);
    assert_eq!(seqs(&by_cumulative), vec![1, 2, 3]);
}
