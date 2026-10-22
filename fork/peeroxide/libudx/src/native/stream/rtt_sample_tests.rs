//! Tests of the RTT sample: every removed never-retransmitted packet (cumulative ACK and SACK), outlier clamp as in C.
use super::*;
use crate::native::header::SackRange;
use crate::native::stream::test_support::{resend_lost_all, stream_inner, with_sent};
use std::time::Duration;

/// Packets `0..n`, the `i`-th has `time_sent` `ages[i]` ms before "now".
fn inner_with_ages(ages: &[u64]) -> StreamInner {
    let mut inner = stream_inner();
    with_sent(&mut inner, 0, ages.len() as u32);
    let now = Instant::now();
    for (i, age) in ages.iter().enumerate() {
        inner
            .outgoing
            .set_time_sent(Seq(i as u32), now - Duration::from_millis(*age));
    }
    inner
}

/// Reference fold of samples by the C formulas: (srtt, rttvar).
fn fold(samples: &[u32]) -> (u32, u32) {
    let (mut srtt, mut var) = (0u32, 0u32);
    for &r in samples {
        if srtt == 0 {
            (srtt, var) = (r, r / 2);
            continue;
        }
        var = (3 * var + srtt.abs_diff(r)) / 4;
        srtt = (7 * srtt + r) / 8;
    }
    (srtt, var)
}

fn sack(start: u32, end: u32) -> Vec<SackRange> {
    vec![SackRange { start, end }]
}

#[test]
fn cumulative_ack_samples_every_unretransmitted_packet() {
    let mut inner = inner_with_ages(&[300, 200, 100]);
    inner.on_cumulative_ack(3);
    let (srtt, var) = fold(&[300, 200, 100]);
    assert!(
        inner.srtt.abs_diff(srtt) <= 3,
        "srtt {} vs {srtt}",
        inner.srtt
    );
    assert!(
        inner.rttvar.abs_diff(var) <= 3,
        "rttvar {} vs {var}",
        inner.rttvar
    );
    assert!(inner.srtt.abs_diff(300) > 10, "not just the first sample");
}

#[test]
fn retransmitted_and_already_sacked_packets_give_no_sample() {
    let mut inner = inner_with_ages(&[300, 200]);
    inner.on_sack(&sack(1, 2));
    assert_eq!(inner.detect_losses(), 1);
    resend_lost_all(&mut inner);
    assert!(inner.outgoing[&Seq(0)].retransmit_count > 0);
    let after_sack = inner.srtt;
    inner.on_cumulative_ack(2);
    assert_eq!(
        inner.srtt, after_sack,
        "neither a retransmitted nor an already SACKed packet gives a sample"
    );
}

#[test]
fn sack_gives_sample_once() {
    let mut inner = inner_with_ages(&[0, 200, 0]);
    inner.on_sack(&sack(1, 2));
    let first = inner.srtt;
    assert!(first.abs_diff(200) <= 3, "srtt {first}");
    inner.on_sack(&sack(1, 2));
    assert_eq!(inner.srtt, first, "a repeated block gives no sample");
    inner.on_cumulative_ack(2);
    // seq 0 removed (sample ~0 ms) and seq 1 (SACKed, no sample): srtt would otherwise change twice.
    let (expect, _) = fold(&[first, 1]);
    assert!(
        inner.srtt.abs_diff(expect) <= 3,
        "srtt {} vs {expect}",
        inner.srtt
    );
}

#[test]
fn sack_of_retransmitted_packet_gives_no_sample() {
    let mut inner = inner_with_ages(&[300, 200, 100]);
    inner.on_sack(&sack(2, 3));
    inner.detect_losses();
    resend_lost_all(&mut inner);
    assert!(inner.outgoing[&Seq(0)].retransmit_count > 0);
    let before = inner.srtt;
    inner.on_sack(&sack(0, 1));
    assert_eq!(
        inner.srtt, before,
        "SACK of a retransmitted packet gives no sample"
    );
}

#[test]
fn outlier_is_clamped_like_c() {
    let mut inner = stream_inner();
    (inner.srtt, inner.rttvar) = (200, 10);
    inner.update_rtt(20_000);
    assert_eq!(inner.srtt, (7 * 200 + 250) / 8);
    (inner.srtt, inner.rttvar) = (200, 10);
    inner.update_rtt(4_000);
    assert_eq!(inner.srtt, (7 * 200 + 4_000) / 8);
}

#[test]
fn first_sample_is_capped_at_max_rtt() {
    let mut inner = stream_inner();
    inner.update_rtt(40_000);
    assert_eq!(inner.srtt, 30_000);
}
