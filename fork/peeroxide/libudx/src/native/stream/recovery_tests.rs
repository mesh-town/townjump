//! Recovery after losses: RACK by send order and RTT sampling.
//! Times are explicit `Instant`s (`set_time_sent`, `send_more_at`), no sleeping.
use super::test_support::{resend_lost_all, send_more_at, stream_inner, with_sent};
use super::*;
use std::time::Duration;

// ── Helpers ─────────────────────────────────────────────────────────────────

/// A stream with `count` packets from number 0 (times are deterministic, see `with_sent`).
fn sent(count: u32) -> StreamInner {
    let mut inner = stream_inner();
    with_sent(&mut inner, 0, count);
    inner
}

/// SACK of one block `[start, end)`.
fn sack(inner: &mut StreamInner, start: u32, end: u32) {
    inner.on_sack(&[SackRange { start, end }]);
}

fn resent_count(inner: &StreamInner, seq: u32) -> u32 {
    inner.outgoing[&Seq(seq)].retransmit_count
}

fn is_lost(inner: &StreamInner, seq: u32) -> bool {
    inner.outgoing[&Seq(seq)].lost
}

/// 10 packets, SACK `[5, sack_end)`, losses 0..4 found and retransmitted: a retransmission was sent later than everything delivered.
fn resent_hole(sack_end: u32) -> StreamInner {
    let mut inner = sent(10);
    sack(&mut inner, 5, sack_end);
    assert_eq!(inner.detect_losses(), 5, "losses 0..4 found");
    assert_eq!(
        resend_lost_all(&mut inner).len(),
        5,
        "losses 0..4 retransmitted"
    );
    inner
}

// ── 1a: RACK by send order ──────────────────────────────────────────────────

mod rack_marks_by_send_order {
    use super::*;

    #[test]
    fn resent_hole_is_not_marked_again_by_100_acks_of_older_packets() {
        // given: 10 packets, SACK [5, 6), losses 0..4 retransmitted
        let mut inner = resent_hole(6);
        // when: 100 ACKs with SACKs of packets sent before the retransmission
        for k in 0..100 {
            sack(&mut inner, 5, 6 + k % 4);
            let lost = inner.detect_losses();
            resend_lost_all(&mut inner);
            // then: the hole is retransmitted once, no new losses
            assert_eq!(lost, 0, "ACK {k}");
        }
        assert_eq!(resent_count(&inner, 0), 1);
        assert_eq!(inner.congestion.lost, 5);
    }

    #[test]
    fn delivery_of_a_packet_sent_after_the_resend_marks_the_hole_again() {
        // given: as in the first, but SACK [5, 10) (otherwise 6..9 would be marked too)
        let mut inner = resent_hole(10);
        // when: a packet sent after the retransmission is delivered
        send_more_at(&mut inner, 1, Instant::now() + Duration::from_secs(1));
        sack(&mut inner, 10, 11);
        let lost = inner.detect_losses();
        resend_lost_all(&mut inner);
        // then: the hole is lost again and retransmitted a second time
        assert_eq!(lost, 5);
        assert_eq!(resent_count(&inner, 0), 2);
    }

    #[test]
    fn marking_needs_no_rtt_sample() {
        // given: no RTT sample
        let mut inner = sent(8);
        assert_eq!(inner.srtt, 0);
        // when
        sack(&mut inner, 5, 7);
        // then: packets 0..4 are lost even without an RTT
        assert_eq!(inner.detect_losses(), 5);
    }

    #[test]
    fn sack_of_a_resent_packet_faster_than_min_rtt_does_not_advance_the_send_order() {
        // given: 6 packets, SACK [5, 6), 0..4 retransmitted; 0 and 1 retransmitted in r1, 2..4 later, in r2
        let mut inner = resent_hole_of_six();
        // when: SACK of retransmitted 3: its RTT 0 is shorter than min RTT 1 ms, the clock does not move (C 1342; otherwise 0 and 1 would be marked again)
        sack(&mut inner, 3, 4);
        // then
        assert_eq!(inner.detect_losses(), 0);
        assert!(!is_lost(&inner, 0) && !is_lost(&inner, 1));
    }

    fn resent_hole_of_six() -> StreamInner {
        let mut inner = sent(6);
        sack(&mut inner, 5, 6);
        assert_eq!(inner.detect_losses(), 5);
        assert_eq!(resend_lost_all(&mut inner).len(), 5);
        let r1 = Instant::now();
        let r2 = r1 + Duration::from_millis(1);
        for seq in 0..5 {
            let at = if seq < 2 { r1 } else { r2 };
            inner.outgoing.set_time_sent(Seq(seq), at);
        }
        inner
    }

    #[test]
    fn equal_send_times_do_not_mark() {
        // given: 2 packets with the same send time
        let mut inner = sent(2);
        let at = Instant::now();
        inner.outgoing.set_time_sent(Seq(0), at);
        inner.outgoing.set_time_sent(Seq(1), at);
        // when: SACK of the second
        sack(&mut inner, 1, 2);
        // then: the first is not lost
        assert_eq!(inner.detect_losses(), 0);
        assert!(!is_lost(&inner, 0));
    }
}

// ── 1b: an RTT sample is not taken from a packet confirmed by SACK ──────────

mod rtt_skips_sacked {
    use super::*;

    /// Send time 5 s ago.
    fn five_seconds_ago() -> Instant {
        Instant::now() - Duration::from_secs(5)
    }

    #[test]
    fn rtt_is_not_sampled_from_a_packet_sacked_earlier() {
        // given: packet 0 retransmitted, packet 1 got a SACK, both sent 5 s ago
        let mut inner = sent(2);
        sack(&mut inner, 1, 2);
        assert_eq!(inner.detect_losses(), 1);
        resend_lost_all(&mut inner);
        for seq in 0..2 {
            inner.outgoing.set_time_sent(Seq(seq), five_seconds_ago());
        }
        // the SACK of packet 1 already gave its sample (as C `ack_packet`) — remember the estimate
        let after_sack = inner.srtt;
        // when: a cumulative ACK takes both
        inner.on_cumulative_ack(2);
        // then: no new sample (retransmission — Karn's rule, the SACKed one waited for the hole)
        assert_eq!(inner.srtt, after_sack);
    }

    #[test]
    fn rtt_is_sampled_from_a_fresh_unsacked_packet() {
        // given: packet 0 got a SACK 5 s ago, packet 1 is fresh
        let mut inner = sent(2);
        sack(&mut inner, 0, 1);
        inner.outgoing.set_time_sent(Seq(0), five_seconds_ago());
        // when: a cumulative ACK takes both
        inner.on_cumulative_ack(2);
        // then: the sample is from packet 1, not 5 s
        assert!(inner.srtt > 0 && inner.srtt < 1000, "srtt {}", inner.srtt);
    }
}
