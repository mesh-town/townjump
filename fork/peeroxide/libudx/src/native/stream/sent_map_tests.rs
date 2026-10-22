//! `SentMap` tests: counters equal a map walk, `detect_losses` equals the old walk, including across 2^32.
use super::*;
use crate::native::header::SackRange;
use crate::native::stream::test_support::{resend_lost_all, rto_resend, stream_inner, with_sent};

/// First number: the flight crosses 2^32 (MAX-3, MAX-2, MAX-1, MAX, 0, 1, ...).
const FIRST: u32 = u32::MAX - 3;

// ── Reference: how it was counted before the counters ───────────────────────

/// The old `StreamInner::inflight`: a walk of the whole map.
fn inflight_by_walk(inner: &StreamInner) -> u32 {
    inner
        .outgoing
        .values()
        .filter(|p| !p.sacked && !p.lost)
        .count() as u32
}

/// Reference for `detect_losses_at`: the numbers RACK will mark at a given time, in map order. The walk continues while a packet
/// is a retransmission or sent before `rack.sent`; packets without SACK and losses before `rack.sent` with a deadline are marked
/// `time_sent + rack.rtt + reo_wnd <= now`.
fn detect_losses_by_walk(inner: &StreamInner, now: Instant) -> Vec<u32> {
    let Some(rack_sent) = inner.outgoing.rack().sent else {
        return Vec::new();
    };
    let wait = inner.outgoing.rack().rtt + inner.reo_wnd();
    inner
        .outgoing
        .iter()
        .take_while(|(_, p)| p.retransmit_count > 0 || p.time_sent < rack_sent)
        .filter(|&(_, p)| !p.sacked && !p.lost && p.time_sent < rack_sent)
        .filter(|&(_, p)| p.time_sent + wait <= now)
        .map(|(&seq, _)| seq.0)
        .collect()
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn sent(count: u32) -> StreamInner {
    let mut inner = stream_inner();
    with_sent(&mut inner, FIRST, count);
    inner
}

/// Number `FIRST + i` with wraparound.
fn nth(i: u32) -> u32 {
    FIRST.wrapping_add(i)
}

/// SACK of blocks `[FIRST+from, FIRST+to)`.
fn sack(inner: &mut StreamInner, blocks: &[(u32, u32)]) {
    let ranges: Vec<SackRange> = blocks
        .iter()
        .map(|&(from, to)| SackRange {
            start: nth(from),
            end: nth(to),
        })
        .collect();
    inner.on_sack(&ranges);
}

/// Numbers with `lost` in map order.
fn lost_seqs(inner: &StreamInner) -> Vec<u32> {
    inner
        .outgoing
        .iter()
        .filter(|(_, p)| p.lost)
        .map(|(seq, _)| seq.0)
        .collect()
}

/// Counters equal a map walk, `inflight()` — the old walk.
fn assert_counters(inner: &StreamInner) {
    let map = &inner.outgoing;
    let counters = (map.inflight(), map.sacked, map.lost, map.highest_sacked);
    assert_eq!(counters, map.recount(), "counters");
    assert_eq!(inner.inflight(), inflight_by_walk(inner), "inflight");
}

/// `detect_losses` against the old walk: the same numbers in the same order and the same result;
/// `inflight` right after it (where `congestion.on_ack` takes it) — as in the walk.
fn detect_and_compare(inner: &mut StreamInner) -> u32 {
    let now = Instant::now();
    let expected = detect_losses_by_walk(inner, now);
    let already = lost_seqs(inner);
    let n = inner.detect_losses_at(now).0;
    let marked: Vec<u32> = lost_seqs(inner)
        .into_iter()
        .filter(|s| !already.contains(s))
        .collect();
    assert_eq!(marked, expected, "marked lost");
    assert_eq!(n as usize, expected.len(), "detect_losses result");
    assert_counters(inner);
    n
}

mod counters_follow_packet_state {
    use super::*;

    #[test]
    fn every_registered_packet_is_in_flight() {
        // given: 8 packets across 2^32
        let inner = sent(8);
        // then
        assert_eq!(inner.inflight(), 8);
        assert_counters(&inner);
    }

    #[test]
    fn sacked_packets_leave_the_flight() {
        // given
        let mut inner = sent(8);
        // when: SACK [5, 7)
        sack(&mut inner, &[(5, 7)]);
        // then
        assert_eq!(inner.inflight(), 6);
        assert_eq!(inner.outgoing.highest_sacked, Some(Seq(nth(6))));
        assert_counters(&inner);
    }

    #[test]
    fn repeated_sack_counts_once() {
        // given: SACK [5, 7)
        let mut inner = sent(8);
        sack(&mut inner, &[(5, 7)]);
        // when: block [4, 7) covers the previous one
        sack(&mut inner, &[(4, 7)]);
        // then
        assert_eq!(inner.outgoing.sacked, 3);
        assert_counters(&inner);
    }

    #[test]
    fn cumulative_ack_over_every_sack_forgets_the_highest() {
        // given: SACK [5, 7) out of 8
        let mut inner = sent(8);
        sack(&mut inner, &[(5, 7)]);
        // when: ack 7 removes 0..7 together with the SACK
        inner.on_cumulative_ack(nth(7));
        // then
        assert_eq!(inner.outgoing.highest_sacked, None);
        assert_eq!(inner.inflight(), 1);
        assert_counters(&inner);
    }

    #[test]
    fn cumulative_ack_below_the_highest_keeps_it() {
        // given: SACK [5, 7) out of 8
        let mut inner = sent(8);
        sack(&mut inner, &[(5, 7)]);
        // when: ack 3
        inner.on_cumulative_ack(nth(3));
        // then: 3, 4, 7 in flight
        assert_eq!(inner.outgoing.highest_sacked, Some(Seq(nth(6))));
        assert_eq!(inner.inflight(), 3);
        assert_counters(&inner);
    }

    #[test]
    fn rto_resend_keeps_the_counters() {
        // given: SACK [5, 7) out of 8
        let mut inner = sent(8);
        sack(&mut inner, &[(5, 7)]);
        // when
        let resent = rto_resend(&mut inner);
        // then: 6 without SACK retransmitted, counters the same
        assert_eq!(resent.len(), 6);
        assert_eq!(inner.inflight(), 6);
        assert_counters(&inner);
    }

    #[test]
    fn direct_insert_is_counted() {
        // given: insertion bypassing register_sent, as in mtu_tests; a packet with SACK — otherwise `count` cannot be checked
        let mut inner = stream_inner();
        let packet = SentPacket {
            packet: vec![0; HEADER_SIZE],
            time_sent: Instant::now(),
            retransmit_count: 0,
            sacked: true,
            is_mtu_probe: false,
            lost: false,
            rate_info: None,
            remote_addr: None,
        };
        // when
        inner.outgoing.insert(Seq(0), packet);
        // then
        assert_eq!(inner.inflight(), 0);
        assert_eq!(inner.outgoing.highest_sacked, Some(Seq(0)));
        assert_counters(&inner);
    }

    #[test]
    fn sack_of_a_lost_packet_clears_the_loss() {
        // given: SACK [5, 7), losses 0..5 before fast retransmission
        let mut inner = sent(8);
        sack(&mut inner, &[(5, 7)]);
        inner.detect_losses();
        // when: SACK covered the lost 2
        sack(&mut inner, &[(2, 3)]);
        // then
        assert!(!inner.outgoing[&Seq(nth(2))].lost);
        assert_eq!(inner.outgoing.lost, 4);
        assert_counters(&inner);
    }
}

mod detect_losses_matches_the_walk {
    use super::*;

    #[test]
    fn without_sack_nothing_is_lost() {
        // given
        let mut inner = sent(8);
        // then
        assert_eq!(detect_and_compare(&mut inner), 0);
    }

    #[test]
    fn packets_below_the_highest_sack_are_lost_across_the_wrap() {
        // given: SACK [5, 7) out of 8
        let mut inner = sent(8);
        sack(&mut inner, &[(5, 7)]);
        // when
        let lost = detect_and_compare(&mut inner);
        // then: MAX-3..0 lost, only 7 in flight
        assert_eq!(lost, 5);
        let expected = vec![u32::MAX - 3, u32::MAX - 2, u32::MAX - 1, u32::MAX, 0];
        assert_eq!(lost_seqs(&inner), expected);
        assert_eq!(inner.inflight(), 1);
    }

    #[test]
    fn several_blocks_leave_every_gap_lost() {
        // given: 12 packets, SACK [2, 4) and [7, 9)
        let mut inner = sent(12);
        sack(&mut inner, &[(2, 4), (7, 9)]);
        // then: 0, 1, 4, 5, 6
        assert_eq!(detect_and_compare(&mut inner), 5);
    }

    #[test]
    fn fast_retransmit_returns_the_lost_to_the_flight() {
        // given: losses 0..5 found
        let mut inner = sent(8);
        sack(&mut inner, &[(5, 7)]);
        detect_and_compare(&mut inner);
        // when
        let resent = resend_lost_all(&mut inner);
        // then
        assert_eq!(resent.len(), 5);
        assert!(lost_seqs(&inner).is_empty());
        assert_eq!(inner.inflight(), 6);
        assert_counters(&inner);
    }

    #[test]
    fn next_ack_does_not_mark_a_resent_hole_again() {
        // given: losses found and retransmitted (the retransmission was sent after everything delivered)
        let mut inner = sent(8);
        sack(&mut inner, &[(5, 7)]);
        detect_and_compare(&mut inner);
        resend_lost_all(&mut inner);
        // when / then: the next ACK without a new SACK does not mark the same hole: there was no delivery after the retransmission
        assert_eq!(detect_and_compare(&mut inner), 0);
    }

    #[test]
    fn cumulative_ack_into_the_hole_shrinks_the_loss() {
        // given: SACK [5, 7), ack 3
        let mut inner = sent(8);
        sack(&mut inner, &[(5, 7)]);
        inner.on_cumulative_ack(nth(3));
        // then: 3 and 4 lost
        assert_eq!(detect_and_compare(&mut inner), 2);
    }

    #[test]
    fn full_ack_path_keeps_the_counters() {
        // given
        let mut inner = sent(8);
        // when: ack 1 and block [5, 7) via on_ack_packet (apply_acks with a cross-check)
        let block = SackRange {
            start: nth(5),
            end: nth(7),
        };
        inner.on_ack_packet(nth(1), DEFAULT_RWND, &[block]);
        // then: 0 removed; 1..5 lost and awaiting retransmission; only 7 in flight (C: a lost one is outside `inflight_queue`)
        assert_eq!(inner.inflight(), 1);
        assert_counters(&inner);
    }
}

mod random_series_matches_the_walk {
    use super::*;

    /// Linear congruential generator: the series is repeatable by seed.
    struct Lcg(u64);

    impl Lcg {
        /// A number from `0..n`, `n > 0`.
        fn below(&mut self, n: u32) -> u32 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((self.0 >> 33) as u32) % n
        }
    }

    fn span(inner: &StreamInner) -> u32 {
        inner.next_seq.wrapping_sub(inner.remote_acked)
    }

    /// A random SACK block inside `[remote_acked, next_seq)`.
    fn random_sack(inner: &mut StreamInner, rng: &mut Lcg) {
        let span = span(inner);
        if span == 0 {
            return;
        }
        let from = rng.below(span);
        let start = inner.remote_acked.wrapping_add(from);
        let end = start.wrapping_add(1 + rng.below(span - from));
        inner.on_sack(&[SackRange { start, end }]);
    }

    /// Cumulative ACK 0..=3 packets forward, not beyond `next_seq`.
    fn random_cumulative_ack(inner: &mut StreamInner, rng: &mut Lcg) {
        let step = rng.below(4).min(span(inner));
        inner.on_cumulative_ack(inner.remote_acked.wrapping_add(step));
    }

    fn send_next(inner: &mut StreamInner) {
        let seq = inner.next_seq;
        inner.register_sent(seq, build_data_packet(inner.remote_id, seq, 0, b"n"), false);
        inner.next_seq = seq.wrapping_add(1);
    }

    fn random_step(inner: &mut StreamInner, rng: &mut Lcg) {
        match rng.below(5) {
            0 => random_sack(inner, rng),
            1 => {
                detect_and_compare(inner);
                resend_lost_all(inner);
            }
            2 => random_cumulative_ack(inner, rng),
            3 => {
                rto_resend(inner);
            }
            _ => send_next(inner),
        }
    }

    #[test]
    fn counters_and_losses_follow_the_walk_for_20_seeds_of_300_steps() {
        for seed in 1..=20 {
            // given: 40 packets from MAX-20 — the series crosses 2^32
            let mut inner = stream_inner();
            with_sent(&mut inner, u32::MAX - 20, 40);
            let mut rng = Lcg(seed);
            // when / then: after every step the counters equal a walk
            for _ in 0..300 {
                random_step(&mut inner, &mut rng);
                assert_counters(&inner);
            }
        }
    }
}
