//! RACK clock tests (`rack.rs`): `clock` — the delivery clock, `time` — RACK by time.
use super::super::*;
use super::Rack;
use crate::native::stream::test_support::{resend_lost_all, stream_inner, with_sent};
use std::time::Duration;

mod clock {
    use super::*;

    fn sack(inner: &mut StreamInner, start: u32, end: u32) {
        inner.on_sack(&[SackRange { start, end }]);
    }

    fn sleep_ms(ms: u64) {
        std::thread::sleep(Duration::from_millis(ms));
    }

    const MS: Duration = Duration::from_millis(1);

    /// Clock after the first delivery with RTT 50 ms: `(rack, now, t)`.
    fn with_min_50() -> (Rack, Instant, Instant) {
        let now = Instant::now();
        let t = now - 50 * MS;
        let mut rack = Rack::default();
        rack.on_delivered(t, false, now);
        (rack, now, t)
    }

    #[test]
    fn first_send_sets_rtt_min_and_moves_the_clock() {
        let (rack, _, t) = with_min_50();
        assert_eq!(rack.sent, Some(t));
        assert_eq!(rack.rtt_min_ms, Some(50));
        assert_eq!(rack.rtt, 50 * MS);
    }

    #[test]
    fn zero_rtt_counts_as_one_ms_for_min() {
        let now = Instant::now();
        let mut rack = Rack::default();
        rack.on_delivered(now, false, now);
        assert_eq!(rack.rtt_min_ms, Some(1));
    }

    #[test]
    fn retransmit_faster_than_min_rtt_keeps_the_clock() {
        let (mut rack, now, t) = with_min_50();
        rack.on_delivered(now - 10 * MS, true, now);
        assert_eq!((rack.sent, rack.rtt), (Some(t), 50 * MS));
    }

    #[test]
    fn retransmit_not_faster_than_min_rtt_moves_the_clock_but_not_min() {
        let (mut rack, now, _) = with_min_50();
        let (later, t2) = (now + 20 * MS, now - 40 * MS);
        rack.on_delivered(t2, true, later);
        assert_eq!((rack.sent, rack.rtt), (Some(t2), 60 * MS));
        assert_eq!(rack.rtt_min_ms, Some(50));
    }

    #[test]
    fn retransmit_before_any_sample_keeps_the_clock() {
        let now = Instant::now();
        let mut rack = Rack::default();
        rack.on_delivered(now - 5 * MS, true, now);
        assert_eq!(rack.sent, None);
    }

    #[test]
    fn older_delivery_updates_rtt_but_not_the_clock() {
        let (mut rack, now, t) = with_min_50();
        rack.on_delivered(t - 20 * MS, false, now);
        assert_eq!(rack.sent, Some(t));
        assert_eq!(rack.rtt, 70 * MS);
    }

    #[test]
    fn acked_seq_compares_modulo_2_32() {
        let mut rack = Rack::default();
        rack.on_acked_seq(Seq(u32::MAX - 1), false);
        rack.on_acked_seq(Seq(u32::MAX), false);
        assert!(!rack.reordering_seen);
        assert_eq!(rack.fack, Some(Seq(0)));
        rack.on_acked_seq(Seq(u32::MAX - 1), false);
        assert!(rack.reordering_seen);
        assert_eq!(rack.fack, Some(Seq(0)));
    }

    #[test]
    fn delivered_resend_moves_the_clock_and_marks_an_older_resend() {
        // given: 10 packets; SACK [1,2) marks 0, it is retransmitted; SACK [3,10) marks 2, it is retransmitted later
        let mut inner = stream_inner();
        with_sent(&mut inner, 0, 10);
        sack(&mut inner, 1, 2);
        assert_eq!(inner.detect_losses(), 1);
        sleep_ms(1);
        assert_eq!(resend_lost_all(&mut inner).len(), 1);
        sack(&mut inner, 3, 10);
        assert_eq!(inner.detect_losses(), 1);
        sleep_ms(1);
        assert_eq!(resend_lost_all(&mut inner).len(), 1);
        sleep_ms(2);
        // when: retransmission 2 is delivered (RTT >= 2 ms >= min 1 ms)
        sack(&mut inner, 2, 3);
        // then: the clock moved past retransmission 0, it is marked again
        assert_eq!(inner.detect_losses(), 1);
        assert!(inner.outgoing[&Seq(0)].lost);
    }
}

mod time {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    /// 10 packets sent at `now − 40 ms + i us`, `srtt = 100`, no SACK yet.
    fn young_sent() -> StreamInner {
        let mut inner = stream_inner();
        with_sent(&mut inner, 0, 10);
        inner.srtt = 100;
        let base = Instant::now() - 40 * MS;
        for i in 0..10 {
            let at = base + Duration::from_micros(u64::from(i));
            inner.outgoing.set_time_sent(Seq(i), at);
        }
        inner
    }

    /// Hole 0..9 younger than `rtt + reo_wnd`: `young_sent()` + SACK [9,10).
    fn young_hole() -> StreamInner {
        let mut inner = young_sent();
        inner.on_sack(&[SackRange { start: 9, end: 10 }]);
        inner
    }

    fn sack(inner: &mut StreamInner, start: u32, end: u32) {
        inner.on_sack(&[SackRange { start, end }]);
    }

    #[test]
    fn young_hole_waits_reo_wnd() {
        // min RTT 40 ms -> reo_wnd 10 ms; hole 40 ms < 40 + 10 ms: waits
        let mut inner = young_hole();
        assert_eq!(inner.detect_losses(), 0);
    }

    #[test]
    fn young_hole_arms_reo_timer_up_to_reo_wnd() {
        let mut inner = young_hole();
        let (lost, timer) = inner.detect_losses_at(Instant::now());
        assert_eq!(lost, 0);
        let d = timer.expect("the hole waits: REO timer armed");
        assert!(d > 5 * MS && d <= 10 * MS, "REO deadline {d:?}");
    }

    #[test]
    fn reo_timer_marks_the_hole_when_it_expires() {
        let mut inner = young_hole();
        std::thread::sleep(11 * MS);
        inner.on_reo_fire();
        assert_eq!(inner.congestion.lost, 9);
    }

    #[test]
    fn three_sacks_zero_reo_wnd() {
        // three SACKs in ascending order: no reordering, the window is cut to 0
        let mut inner = young_sent();
        sack(&mut inner, 7, 8);
        assert!(
            inner.reo_wnd() > Duration::ZERO,
            "one SACK: the window holds"
        );
        sack(&mut inner, 8, 10);
        assert!(!inner.outgoing.rack().reordering_seen);
        assert_eq!(inner.reo_wnd(), Duration::ZERO);
    }

    #[test]
    fn reordering_flag_set_by_lower_nonretransmitted_ack() {
        // a) SACK 9, then 7..9 below it: reordering, the window holds with 3 SACKs
        let mut inner = young_hole();
        sack(&mut inner, 7, 9);
        assert!(inner.outgoing.rack().reordering_seen);
        assert!(inner.reo_wnd() > Duration::ZERO);
        // b) the lower packet was resent: not reordering
        let mut inner = stream_inner();
        with_sent(&mut inner, 0, 10);
        sack(&mut inner, 9, 10);
        std::thread::sleep(2 * MS);
        assert!(inner.detect_losses() > 0);
        resend_lost_all(&mut inner);
        assert!(inner.outgoing[&Seq(7)].retransmit_count > 0);
        sack(&mut inner, 7, 8);
        assert!(!inner.outgoing.rack().reordering_seen);
        // c) ascending order: not reordering
        let mut inner = stream_inner();
        with_sent(&mut inner, 0, 10);
        sack(&mut inner, 7, 8);
        sack(&mut inner, 9, 10);
        assert!(!inner.outgoing.rack().reordering_seen);
    }

    #[test]
    fn reordering_flag_set_by_cumulative_ack_of_unretransmitted_hole() {
        let mut inner = young_hole();
        inner.on_cumulative_ack(9);
        assert!(inner.outgoing.rack().reordering_seen);
    }

    #[test]
    fn reo_wnd_with_reordering_seen_in_recovery() {
        let mut inner = young_hole();
        inner.outgoing.rack_mut().reordering_seen = true;
        inner.congestion.ca_state = CaState::Recovery;
        inner.srtt = 100;
        inner.outgoing.rack_mut().rtt_min_ms = Some(40);
        assert_eq!(inner.reo_wnd(), 10 * MS);
        inner.srtt = 5;
        assert_eq!(inner.reo_wnd(), 5 * MS);
        inner.outgoing.rack_mut().reordering_seen = false;
        assert_eq!(inner.reo_wnd(), Duration::ZERO);
    }

    #[test]
    fn reo_wnd_is_zero_outside_open() {
        let mut inner = young_hole();
        inner.congestion.ca_state = CaState::Recovery;
        assert_eq!(inner.reo_wnd(), Duration::ZERO);
    }

    #[test]
    fn hole_above_highest_sack_sent_before_a_delivered_resend_is_marked() {
        // given: SACK [1,2) marks 0, it is retransmitted; the retransmission is delivered (RTT >= min), highest SACK — 1
        let mut inner = stream_inner();
        with_sent(&mut inner, 0, 10);
        inner.on_sack(&[SackRange { start: 1, end: 2 }]);
        assert_eq!(inner.detect_losses(), 1);
        std::thread::sleep(MS);
        assert_eq!(resend_lost_all(&mut inner).len(), 1);
        std::thread::sleep(2 * MS);
        inner.on_sack(&[SackRange { start: 0, end: 1 }]);
        // then: packets 2..9 are above the highest SACK, but sent before the delivered retransmission: marked
        assert_eq!(inner.detect_losses(), 8);
        assert!(inner.outgoing[&Seq(9)].lost);
    }
}
