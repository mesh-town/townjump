//! RTO marking tests (`mark_rto_lost`, `cut_cwnd_to_flight`) and RTO re-arm on delivery: RTO marks the overdue ones,
//! cuts `cwnd` to "in flight + 1", retransmission goes on a kick (C `udx_rto_timeout`, udx.c:1163-1235).
use super::*;
use crate::native::congestion::CaState;
use crate::native::header::SackRange;
use crate::native::stream::test_support::{stream_inner, with_sent};
use crate::native::stream::{DEFAULT_RWND, Seq};
use std::time::{Duration, Instant};

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn sack(inner: &mut StreamInner, start: u32, end: u32) {
    inner.on_sack(&[SackRange { start, end }]);
}

mod rto_marks {
    use super::*;

    #[test]
    fn rto_marks_all_overdue_and_cuts_cwnd_to_one() {
        // given: 8 packets in flight
        let mut inner = stream_inner();
        with_sent(&mut inner, 0, 8);
        // when: RTO
        let next = inner.on_rto_fire();
        // then: all marked, none in flight, window 1, nothing sent
        assert_eq!(next, Some(ms(2000)));
        assert_eq!(inner.inflight(), 0);
        assert_eq!(inner.congestion.lost, 8);
        assert_eq!(inner.congestion.cwnd, 1);
        assert_eq!(inner.outgoing.resent(), 0);
        assert_eq!(inner.congestion.ca_state, CaState::Loss);
    }

    #[test]
    fn rto_spares_packets_younger_than_rack_rtt() {
        // given: 6 packets; 0-3 sent 1 s ago, 4 — 300 ms, 5 — 100 ms; SACK 4 (rack.rtt ≈ 300 ms)
        let mut inner = stream_inner();
        with_sent(&mut inner, 0, 6);
        let now = Instant::now();
        for (seq, age) in [
            (0, 1000),
            (1, 1000),
            (2, 1000),
            (3, 1000),
            (4, 300),
            (5, 100),
        ] {
            inner.outgoing.set_time_sent(Seq(seq), now - ms(age));
        }
        sack(&mut inner, 4, 5);
        // when: RTO
        inner.on_rto_fire();
        // then: 0-3 lost; 5 is younger than rack.rtt — in flight
        let lost: Vec<u32> = (0..6).filter(|&s| inner.outgoing[&Seq(s)].lost).collect();
        assert_eq!(lost, vec![0, 1, 2, 3]);
        assert_eq!(inner.inflight(), 1);
        assert_eq!(inner.congestion.cwnd, 2);
    }

    #[test]
    fn rto_marks_the_head_even_when_young() {
        // given: 4 packets, seq 3 sent 300 ms ago and delivered by SACK
        let mut inner = stream_inner();
        with_sent(&mut inner, 0, 4);
        inner
            .outgoing
            .set_time_sent(Seq(3), Instant::now() - ms(300));
        sack(&mut inner, 3, 4);
        // when: RTO
        inner.on_rto_fire();
        // then: only the head is lost, 1 and 2 are younger than rack.rtt
        let lost: Vec<u32> = (0..4).filter(|&s| inner.outgoing[&Seq(s)].lost).collect();
        assert_eq!(lost, vec![0]);
        assert_eq!(inner.inflight(), 2);
        assert_eq!(inner.congestion.cwnd, 3);
    }

    #[test]
    fn second_rto_does_not_count_waiting_losses_again() {
        // given: 8 packets, the first RTO marked them all
        let mut inner = stream_inner();
        with_sent(&mut inner, 0, 8);
        inner.on_rto_fire();
        // when: a second RTO with no retransmissions between them
        inner.on_rto_fire();
        // then: losses are not counted again
        assert_eq!(inner.congestion.lost, 8);
    }
}

mod rto_rearm {
    use super::*;

    #[test]
    fn sack_delivery_without_cumulative_advance_rearms_rto() {
        // given: 3 packets in flight, head lost
        let mut inner = stream_inner();
        with_sent(&mut inner, 0, 3);
        // when: ACK with SACK [2, 3) without moving the cumulative ACK
        let outcome = inner.on_ack_packet(0, DEFAULT_RWND, &[SackRange { start: 2, end: 3 }]);
        // then: C re-arms RTO on any delivery (udx.c:1751-1765)
        assert_eq!(outcome.rto_timer, Some(single_rto(inner.rto)));
    }

    #[test]
    fn repeated_sack_without_delivery_leaves_rto_alone() {
        // given: SACK [2, 3) already counted
        let mut inner = stream_inner();
        with_sent(&mut inner, 0, 3);
        inner.on_ack_packet(0, DEFAULT_RWND, &[SackRange { start: 2, end: 3 }]);
        // when: the same SACK again
        let outcome = inner.on_ack_packet(0, DEFAULT_RWND, &[SackRange { start: 2, end: 3 }]);
        // then: nothing new delivered — leave RTO alone
        assert_eq!(outcome.rto_timer, None);
    }
}
