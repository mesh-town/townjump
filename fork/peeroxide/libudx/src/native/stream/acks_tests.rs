//! Incoming ACK tests: which timers to touch (RTO, ZWP), window, D6 after a cumulative ack, pacing wakeup.
use crate::native::header::SackRange;
use crate::native::stream::test_support::{
    stream_inner, with_pending_write, with_queued, with_sent,
};
use crate::native::stream::{DEFAULT_RWND, FAR_FUTURE, StreamInner};
use std::time::Duration;

mod window_only_ack {
    use super::*;

    // ack 0 and no SACK — window update only

    #[test]
    fn stores_the_advertised_window() {
        // given: two packets in flight
        let mut inner = sent_stream(2);
        // when
        inner.on_ack_packet(0, 1234, &[]);
        // then
        assert_eq!(inner.send_rwnd, 1234);
    }

    #[test]
    fn leaves_rto_timer_alone() {
        // given
        let mut inner = sent_stream(2);
        // when
        let outcome = inner.on_ack_packet(0, DEFAULT_RWND, &[]);
        // then
        assert_eq!(outcome.rto_timer, None);
    }

    #[test]
    fn completes_no_writes() {
        // given: a write is pending with threshold 1
        let mut inner = sent_stream(2);
        let _write = with_pending_write(&mut inner, 1);
        // when
        let outcome = inner.on_ack_packet(0, DEFAULT_RWND, &[]);
        // then
        assert!(outcome.completed.is_empty());
    }

    #[test]
    fn wakes_pacing_when_queue_is_not_empty() {
        // given: an unsent packet in the queue
        let mut inner = stream_inner();
        with_queued(&mut inner, 0);
        // when
        let outcome = inner.on_ack_packet(0, DEFAULT_RWND, &[]);
        // then
        assert!(outcome.kick_pacing);
    }

    #[test]
    fn does_not_wake_pacing_when_queue_is_empty() {
        // given
        let mut inner = sent_stream(2);
        // when
        let outcome = inner.on_ack_packet(0, DEFAULT_RWND, &[]);
        // then
        assert!(!outcome.kick_pacing);
    }
}

mod cumulative_ack {
    use super::*;

    #[test]
    fn covering_everything_sent_stops_rto_timer() {
        // given: seq 0 and 1 in flight
        let mut inner = sent_stream(2);
        // when
        let outcome = inner.on_ack_packet(2, DEFAULT_RWND, &[]);
        // then
        assert_eq!(outcome.rto_timer, Some(FAR_FUTURE));
    }

    #[test]
    fn covering_part_restarts_rto_timer_at_one_rto() {
        // given
        let mut inner = sent_stream(2);
        // when: only seq 0 is acked
        let outcome = inner.on_ack_packet(1, DEFAULT_RWND, &[]);
        // then
        let rto = Duration::from_millis(u64::from(inner.rto));
        assert_eq!(outcome.rto_timer, Some(rto));
    }

    #[test]
    fn completes_writes_up_to_the_ack() {
        // given: writes with thresholds 1 and 2
        let mut inner = sent_stream(2);
        let _first = with_pending_write(&mut inner, 1);
        let _second = with_pending_write(&mut inner, 2);
        // when
        let outcome = inner.on_ack_packet(1, DEFAULT_RWND, &[]);
        // then
        assert_eq!(outcome.completed.len(), 1);
    }
}

mod sack_only_ack {
    use super::*;

    #[test]
    fn rearms_rto_timer_at_one_rto_on_delivery() {
        // given: seq 0..5 in flight
        let mut inner = sent_stream(6);
        // when: ack does not move, block [0,2) delivers two packets
        let outcome = inner.on_ack_packet(0, DEFAULT_RWND, &[block(0, 2)]);
        // then: C re-arms RTO on any delivery (udx.c:1751-1765)
        assert_eq!(
            outcome.rto_timer,
            Some(Duration::from_millis(u64::from(inner.rto)))
        );
    }

    #[test]
    fn marks_the_block_packets() {
        // given
        let mut inner = sent_stream(6);
        // when
        inner.on_ack_packet(0, DEFAULT_RWND, &[block(0, 2)]);
        // then
        assert_eq!(sacked_seqs(&inner), vec![0, 1]);
    }
}

mod ack_and_block_in_one_packet {
    use super::*;

    #[test]
    fn block_starting_at_new_ack_is_applied() {
        // given: seq 0..5 in flight
        let mut inner = sent_stream(6);
        // when: ack 2 and block [2,4)
        inner.on_ack_packet(2, DEFAULT_RWND, &[block(2, 4)]);
        // then
        assert_eq!(sacked_seqs(&inner), vec![2, 3]);
    }

    #[test]
    fn block_starting_below_new_ack_is_skipped() {
        // given: seq 0..5 in flight; before ack 2 block [1,3) would be valid (remote_acked 0)
        let mut inner = sent_stream(6);
        // when: ack 2 and block [1,3) — D6 checks after the cumulative ack
        inner.on_ack_packet(2, DEFAULT_RWND, &[block(1, 3)]);
        // then
        assert_eq!(sacked_seqs(&inner), Vec::<u32>::new());
    }
}

mod zero_window_probing {
    use super::*;

    mod window_opens_while_probing {
        use super::*;

        #[test]
        fn stops_the_zwp_timer() {
            // given: ZWP runs with window 0
            let mut inner = probing_stream();
            // when
            let outcome = inner.on_ack_packet(0, DEFAULT_RWND, &[]);
            // then
            assert_eq!(outcome.zwp_timer, Some(FAR_FUTURE));
            assert!(!inner.zwp_active, "ZWP off");
        }
    }

    mod window_stays_closed_while_probing {
        use super::*;

        #[test]
        fn leaves_the_zwp_timer_alone() {
            // given
            let mut inner = probing_stream();
            // when
            let outcome = inner.on_ack_packet(0, 0, &[]);
            // then
            assert_eq!(outcome.zwp_timer, None);
            assert!(inner.zwp_active, "ZWP continues");
        }
    }

    mod full_ack_with_closed_window_and_queued_data {
        use super::*;

        #[test]
        fn arms_the_zwp_timer_at_one_rto_without_probe() {
            // given: seq 0 in flight, seq 1 waits in the queue
            let mut inner = sent_stream(1);
            with_queued(&mut inner, 1);
            // when: ack 1 confirmed everything sent, window 0
            let outcome = inner.on_ack_packet(1, 0, &[]);
            // then
            let rto = Duration::from_millis(u64::from(inner.rto));
            assert_eq!(outcome.zwp_timer, Some(rto));
            assert!(inner.zwp_active, "ZWP on");
        }

        #[test]
        fn also_stops_the_rto_timer() {
            // given
            let mut inner = sent_stream(1);
            with_queued(&mut inner, 1);
            // when
            let outcome = inner.on_ack_packet(1, 0, &[]);
            // then
            assert_eq!(outcome.rto_timer, Some(FAR_FUTURE));
        }
    }

    mod partial_ack_with_closed_window {
        use super::*;

        #[test]
        fn does_not_arm_the_zwp_timer() {
            // given: seq 0 and 1 in flight, data in the queue
            let mut inner = sent_stream(2);
            with_queued(&mut inner, 2);
            // when: only seq 0 is acked, window 0
            let outcome = inner.on_ack_packet(1, 0, &[]);
            // then
            assert_eq!(outcome.zwp_timer, None);
            assert!(
                !inner.zwp_active,
                "packets still in flight — too early for ZWP"
            );
        }
    }
}

// ── setup and helpers ────────────────────────────────────────────────────────

/// A stream with `count` sent DATA packets, starting at seq 0 (`remote_acked` = 0).
fn sent_stream(count: u32) -> StreamInner {
    let mut inner = stream_inner();
    with_sent(&mut inner, 0, count);
    inner
}

/// Window 0 and ZWP enabled.
fn probing_stream() -> StreamInner {
    let mut inner = stream_inner();
    inner.send_rwnd = 0;
    inner.zwp_active = true;
    inner
}

fn block(start: u32, end: u32) -> SackRange {
    SackRange { start, end }
}

/// Sequence numbers of `outgoing` packets marked as received via SACK, ascending.
fn sacked_seqs(inner: &StreamInner) -> Vec<u32> {
    let mut sacked: Vec<u32> = inner
        .outgoing
        .iter()
        .filter(|(_, packet)| packet.sacked)
        .map(|(seq, _)| seq.0)
        .collect();
    sacked.sort_unstable();
    sacked
}
