// JS: libudx/src/udx.c (UDX_HIGH_WATERMARK, send_window_in_bytes, on_bytes_acked)
//! Write budget tests: what counts in the queue, headroom, release by SACK and writer wakeup.

use super::super::test_support::{
    resend_lost_all, stream_inner, with_pending_write, with_queued, with_sent,
};
use super::*;
use crate::native::header::SackRange;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::Wake;
use std::time::Duration;

const B0: usize = 262_144;

#[derive(Default)]
struct Count(AtomicUsize);

impl Wake for Count {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

fn counting_waker() -> (Arc<Count>, Waker) {
    let count = Arc::new(Count::default());
    (count.clone(), Waker::from(count))
}

fn woken(count: &Count) -> usize {
    count.0.load(Ordering::SeqCst)
}

/// Like `with_sent`, but each packet's payload is `max_payload()` bytes.
fn sent_full(inner: &mut StreamInner, first: u32, count: u32) {
    let payload = vec![0u8; inner.max_payload()];
    for i in 0..count {
        let seq = first.wrapping_add(i);
        inner.register_sent(
            seq,
            build_data_packet(inner.remote_id, seq, 0, &payload),
            false,
        );
    }
    inner.remote_acked = first;
    inner.next_seq = first.wrapping_add(count);
}

/// A stream with given windows: `cwnd` packets and `send_rwnd` bytes.
fn stream_with(cwnd: u32, rwnd: u32) -> StreamInner {
    let mut inner = stream_inner();
    inner.congestion.cwnd = cwnd;
    inner.send_rwnd = rwnd;
    inner
}

/// "Full": packets sent up to the budget, plus `extra` on top.
fn fill(inner: &mut StreamInner, extra: usize) -> u32 {
    let mss = inner.max_payload();
    let count = (inner.write_budget().div_ceil(mss) + extra) as u32;
    sent_full(inner, 0, count);
    count
}

fn sack(start: u32, end: u32) -> Vec<SackRange> {
    vec![SackRange { start, end }]
}

mod queued {
    use super::*;

    #[test]
    fn unsent_packets_count_payload_without_header() {
        let mut inner = stream_inner();
        (0..3).for_each(|seq| with_queued(&mut inner, seq));
        assert_eq!(inner.write_queued(), 3);
    }

    #[test]
    fn sent_unacked_packets_count() {
        let mut inner = stream_inner();
        with_sent(&mut inner, 0, 5);
        assert_eq!(inner.write_queued(), 5);
    }

    #[test]
    fn lost_packets_stay_counted() {
        let mut inner = stream_inner();
        with_sent(&mut inner, 0, 5);
        let far = Instant::now() + Duration::from_secs(3600);
        inner.outgoing.mark_rto_lost(Duration::ZERO, far);
        assert_eq!((inner.outgoing.lost(), inner.outgoing.inflight()), (5, 0));
        assert_eq!(inner.write_queued(), 5);
        resend_lost_all(&mut inner);
        assert_eq!(inner.write_queued(), 5);
    }

    #[test]
    fn repeated_sack_subtracts_once() {
        let mut inner = stream_inner();
        with_sent(&mut inner, 0, 5);
        inner.on_ack_packet(0, DEFAULT_RWND, &sack(1, 3));
        assert_eq!(inner.write_queued(), 3);
        inner.on_ack_packet(0, DEFAULT_RWND, &sack(1, 3));
        assert_eq!(inner.write_queued(), 3);
    }

    #[test]
    fn cumulative_ack_after_sack_subtracts_only_the_rest() {
        let mut inner = stream_inner();
        with_sent(&mut inner, 0, 5);
        inner.on_ack_packet(0, DEFAULT_RWND, &sack(1, 3));
        inner.on_ack_packet(3, DEFAULT_RWND, &[]);
        assert_eq!(inner.write_queued(), 2);
    }
}

mod budget {
    use super::*;

    #[test]
    fn fresh_stream_room_is_watermark_plus_initial_window() {
        let inner = stream_with(10, DEFAULT_RWND);
        assert_eq!(inner.write_room(), B0 + 10 * inner.max_payload());
    }

    #[test]
    fn room_follows_cwnd() {
        let inner = stream_with(1000, DEFAULT_RWND);
        assert_eq!(inner.write_room(), B0 + 1000 * inner.max_payload());
    }

    #[test]
    fn room_follows_rwnd_when_smaller() {
        let inner = stream_with(1000, 5000);
        assert_eq!(inner.write_room(), B0 + 5000);
    }

    #[test]
    fn zero_rwnd_leaves_the_watermark() {
        let inner = stream_with(10, 0);
        assert_eq!(inner.write_room(), B0);
    }

    #[test]
    fn probe_rtt_cwnd_four() {
        let inner = stream_with(4, DEFAULT_RWND);
        assert_eq!(inner.write_room(), B0 + 4 * inner.max_payload());
    }

    #[test]
    fn room_follows_mtu() {
        let mut inner = stream_with(10, DEFAULT_RWND);
        inner.mtu = 1500;
        assert_eq!(inner.write_room(), B0 + 10 * (1500 - inner.ip_overhead()));
    }

    #[test]
    // On a 64-bit build `u32::MAX · mss` does not overflow `usize`: we check the `min` of two windows, not saturation.
    fn huge_windows_take_the_smaller() {
        let inner = stream_with(u32::MAX, u32::MAX);
        assert_eq!(inner.write_room(), B0 + u32::MAX as usize);
    }

    #[test]
    fn queue_above_budget_gives_zero_room() {
        let mut inner = stream_with(10, DEFAULT_RWND);
        fill(&mut inner, 50);
        assert_eq!(inner.write_room(), 0);
    }
}

mod sack_frees {
    use super::*;

    #[test]
    fn sack_opens_room_with_head_missing_and_write_pending() {
        let mut inner = stream_with(20_000, DEFAULT_RWND);
        sent_full(&mut inner, 0, 4000);
        let mut rx = with_pending_write(&mut inner, 4000);
        let mss = inner.max_payload();
        assert_eq!(inner.write_room(), 0);

        let outcome = inner.on_ack_packet(0, DEFAULT_RWND, &sack(1, 4000));

        assert_eq!(inner.write_queued(), mss);
        assert!(inner.write_room() >= B0 - mss);
        assert_eq!(inner.remote_acked, 0);
        assert!(outcome.completed.is_empty());
        assert!(rx.try_recv().is_err());
    }
}

mod writer_wake {
    use super::*;

    #[test]
    fn opening_ack_hands_out_the_parked_writer_once() {
        let mut inner = stream_with(10, DEFAULT_RWND);
        let count = fill(&mut inner, 0);
        let (hits, waker) = counting_waker();
        inner.park_writer(&waker);

        let outcome = inner.on_ack_packet(0, DEFAULT_RWND, &sack(1, count));
        outcome.writer.expect("writer handed out").wake();

        assert_eq!(woken(&hits), 1);
        let again = inner.on_ack_packet(0, DEFAULT_RWND, &sack(1, count));
        assert!(again.writer.is_none());
    }

    #[test]
    fn sack_with_head_missing_hands_out_the_writer_once() {
        let mut inner = stream_with(10, DEFAULT_RWND);
        let count = fill(&mut inner, 0);
        let mut head = with_pending_write(&mut inner, count);
        let (hits, waker) = counting_waker();
        inner.park_writer(&waker);

        // ack = 0, the head (0 and 1) does not arrive: no advance, only SACK
        let outcome = inner.on_ack_packet(0, DEFAULT_RWND, &sack(2, count));
        outcome.writer.expect("writer handed out").wake();

        assert_eq!(woken(&hits), 1);
        assert_eq!(inner.remote_acked, 0);
        assert!(outcome.completed.is_empty());
        assert!(head.try_recv().is_err());
        let again = inner.on_ack_packet(0, DEFAULT_RWND, &sack(2, count));
        assert!(again.writer.is_none());
    }

    #[test]
    fn ack_that_leaves_no_room_keeps_the_writer_parked() {
        // The receive window is narrow and comes in every ACK: BBR's cwnd growth will not raise the budget.
        let mut inner = stream_with(10, 0);
        let rwnd = 2 * inner.max_payload() as u32;
        inner.send_rwnd = rwnd;
        let count = fill(&mut inner, 50);
        let (_hits, waker) = counting_waker();
        inner.park_writer(&waker);

        let first = inner.on_ack_packet(0, rwnd, &sack(1, 2));
        assert!(first.writer.is_none());

        let second = inner.on_ack_packet(0, rwnd, &sack(2, count));
        assert!(second.writer.is_some());
    }

    #[test]
    fn no_parked_writer_gives_none() {
        let mut inner = stream_inner();
        with_sent(&mut inner, 0, 5);
        let outcome = inner.on_ack_packet(0, DEFAULT_RWND, &sack(1, 3));
        assert!(outcome.writer.is_none());
    }

    #[test]
    fn window_update_from_zero_wakes() {
        let mut inner = stream_with(10, 0);
        let packets = B0.div_ceil(inner.max_payload()) as u32;
        sent_full(&mut inner, 0, packets);
        let (_hits, waker) = counting_waker();
        inner.park_writer(&waker);
        assert_eq!(inner.write_room(), 0);

        let outcome = inner.on_ack_packet(0, DEFAULT_RWND, &[]);

        assert!(outcome.writer.is_some());
    }

    #[test]
    fn second_park_replaces_the_first() {
        let mut inner = stream_with(10, DEFAULT_RWND);
        let count = fill(&mut inner, 0);
        let (first, w1) = counting_waker();
        let (second, w2) = counting_waker();
        inner.park_writer(&w1);
        inner.park_writer(&w2);

        let outcome = inner.on_ack_packet(0, DEFAULT_RWND, &sack(1, count));
        outcome.writer.expect("writer handed out").wake();

        assert_eq!((woken(&second), woken(&first)), (1, 0));
    }

    #[test]
    fn terminate_wakes_the_parked_writer() {
        let mut inner = stream_inner();
        let (hits, waker) = counting_waker();
        inner.park_writer(&waker);
        inner.terminate();
        assert_eq!(woken(&hits), 1);
    }
}

mod writable {
    use super::*;

    fn same_refusal(inner: &mut StreamInner) {
        let checked = inner.check_writable().expect_err("rejected").to_string();
        let prepared = inner.prepare_write(1).err().expect("rejected").to_string();
        assert_eq!(checked, prepared);
    }

    #[test]
    fn check_writable_rejects_what_prepare_write_rejects() {
        let mut fresh = stream_inner();
        same_refusal(&mut fresh);

        let mut no_udp = stream_inner();
        no_udp.connected = true;
        same_refusal(&mut no_udp);

        let mut ended = stream_inner();
        ended.connected = true;
        ended.ended = true;
        same_refusal(&mut ended);
    }
}
