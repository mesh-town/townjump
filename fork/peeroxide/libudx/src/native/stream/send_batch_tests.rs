//! `take_send_batch` tests: where the wakeup stops, the queue byte sum, the window without double counting.
use super::*;
use crate::native::stream::test_support::{rto_resend, stream_inner};

/// Length of a full DATA: header + `max_payload()` (no address — IPv6 reserve: 20 + 1132 = 1152).
fn full_len(inner: &StreamInner) -> usize {
    HEADER_SIZE + inner.max_payload()
}

/// Puts `count` full DATA (length `full_len`) with `next_seq` via `queue_for_send`.
fn queue_full(inner: &mut StreamInner, count: u32) {
    let payload = vec![0xABu8; inner.max_payload()];
    let packets = (0..count)
        .map(|i| {
            let seq = inner.next_seq.wrapping_add(i);
            (seq, build_data_packet(inner.remote_id, seq, 0, &payload))
        })
        .collect();
    inner.queue_for_send(packets, "127.0.0.1:9".parse().unwrap());
    inner.next_seq = inner.next_seq.wrapping_add(count);
}

// ── loop extracted from incoming.rs ─────────────────────────────────────────

mod batch_stops {
    use super::*;

    #[test]
    fn when_the_queue_empties() {
        // given: 3 packets, window 10
        let mut inner = stream_inner();
        queue_full(&mut inner, 3);
        // when
        let batch = inner.take_send_batch();
        // then: all went out, no delay
        assert_eq!(batch.packets.len(), 3);
        assert!(batch.pacing_delay.is_none());
        assert!(inner.send_queue.is_empty());
        assert_eq!(inner.outgoing.len(), 3);
    }

    #[test]
    fn at_a_zero_window_without_sending() {
        // given: receiver window 0
        let mut inner = stream_inner();
        inner.send_rwnd = 0;
        queue_full(&mut inner, 3);
        // when
        let batch = inner.take_send_batch();
        // then
        assert!(batch.packets.is_empty());
        assert!(batch.pacing_delay.is_none());
        assert_eq!(inner.send_queue.len(), 3);
    }

    #[test]
    fn at_pacing_and_reports_the_delay() {
        // given: pacing 1 B/ms, initial allowance 25 000 B, the window does not interfere
        let mut inner = stream_inner();
        inner.congestion.pacing.set_rate(1);
        inner.congestion.cwnd = 1000;
        queue_full(&mut inner, 100);
        // when
        let batch = inner.take_send_batch();
        // then: the allowance ran out before the queue, the delay is the remainder of the millisecond
        assert!(batch.packets.len() < 100);
        let delay = batch.pacing_delay.expect("stopped on pacing");
        assert!(delay > Duration::ZERO && delay <= Duration::from_millis(1));
    }
}

// ── queue byte sum ──────────────────────────────────────────────────────────

mod queued_bytes {
    use super::*;

    #[test]
    fn follow_push_and_pop() {
        // given: 3 full packets
        let mut inner = stream_inner();
        queue_full(&mut inner, 3);
        assert_eq!(inner.send_queue.bytes(), 3 * full_len(&inner));
        // when: one taken
        inner.send_queue.pop_front();
        // then
        assert_eq!(inner.send_queue.bytes(), 2 * full_len(&inner));
        inner.send_queue.debug_check();
    }

    #[test]
    fn ignore_the_probe_padding() {
        // given: the first DATA will become an MTU probe longer than a full packet
        let mut inner = stream_inner();
        queue_full(&mut inner, 2);
        // when
        let batch = inner.take_send_batch();
        // then: the queued lengths are subtracted, the sum is 0 without overflow
        assert!(batch.packets[0].0.len() > full_len(&inner));
        assert_eq!(inner.send_queue.bytes(), 0);
    }

    #[test]
    fn survive_readdress() {
        // given: packets to the stub 0.0.0.0:0
        let mut inner = stream_inner();
        let sentinel: SocketAddr = "0.0.0.0:0".parse().unwrap();
        inner.queue_for_send(vec![(0, vec![0; 30]), (1, vec![0; 40])], sentinel);
        // when
        inner
            .send_queue
            .readdress(sentinel, "127.0.0.1:9".parse().unwrap());
        // then
        assert!(inner.send_queue.iter().all(|q| q.remote_addr != sentinel));
        assert_eq!(inner.send_queue.bytes(), 70);
    }
}

// ── window without double counting ──────────────────────────────────────────

mod window {
    use super::*;
    use crate::native::stream::test_support::with_sent;

    #[test]
    fn empty_flight_takes_the_whole_initial_cwnd() {
        // given: 12 in the queue, nothing in flight, cwnd 10
        let mut inner = stream_inner();
        queue_full(&mut inner, 12);
        // when
        let batch = inner.take_send_batch();
        // then: 10 (the window is not charged twice for the batch itself)
        assert_eq!(batch.packets.len(), 10);
        assert_eq!(inner.inflight(), 10);
    }

    #[test]
    fn partial_flight_takes_the_rest() {
        // given: 4 in flight, 10 in the queue
        let mut inner = stream_inner();
        with_sent(&mut inner, 0, 4);
        queue_full(&mut inner, 10);
        // when
        let batch = inner.take_send_batch();
        // then: 6 (10 - 4 in flight)
        assert_eq!(batch.packets.len(), 6);
        assert_eq!(inner.send_queue.len(), 4);
    }
}

// ── "nothing to send" input — queue + unacked bytes without SACK ─────────────

mod app_limited {
    use super::*;
    use crate::native::stream::test_support::with_queued;

    /// Three full packets went in flight, one byte follows in the queue (its number is after the three).
    fn three_in_flight_and_a_tail(inner: &mut StreamInner) {
        queue_full(inner, 3);
        inner.take_send_batch();
        with_queued(inner, inner.next_seq);
        inner.congestion.app_limited = 0;
    }

    #[test]
    fn small_queue_with_unacked_full_packets_is_not_app_limited() {
        // given: three full packets in flight, one byte in the queue
        let mut inner = stream_inner();
        three_in_flight_and_a_tail(&mut inner);
        // when
        inner.take_send_batch();
        // then: unacked bytes do not allow entering app-limited
        assert_eq!(inner.congestion.app_limited, 0);
    }

    #[test]
    fn sacked_flight_does_not_hold_back_the_mark() {
        // given: the same three packets, all with SACK
        let mut inner = stream_inner();
        three_in_flight_and_a_tail(&mut inner);
        for seq in 0..3 {
            inner.outgoing.mark_sacked(Seq(seq), Instant::now());
        }
        // when
        inner.take_send_batch();
        // then: the SACKed ones are not in the input, the mark is set
        assert_ne!(inner.congestion.app_limited, 0);
    }

    #[test]
    fn small_queue_on_empty_flight_is_app_limited() {
        // given: nothing in flight, one byte in the queue
        let mut inner = stream_inner();
        with_queued(&mut inner, 0);
        inner.congestion.app_limited = 0;
        // when
        inner.take_send_batch();
        // then
        assert_ne!(inner.congestion.app_limited, 0);
    }

    #[test]
    fn resend_drops_probe_padding_from_unsacked_bytes() {
        // given: two packets in flight, the first an MTU probe with padding
        let mut inner = stream_inner();
        queue_full(&mut inner, 2);
        let batch = inner.take_send_batch();
        assert!(batch.packets[0].0.len() > full_len(&inner));
        // when: an RTO retransmission removes the padding
        rto_resend(&mut inner);
        // then: the sum is by packet lengths in the map, the cross-check is silent
        let sum: usize = inner.outgoing.values().map(|p| p.packet.len()).sum();
        assert_eq!(inner.outgoing.unsacked_bytes(), sum);
        assert_eq!(sum, 2 * full_len(&inner));
        inner.outgoing.debug_check();
    }
}

mod pace_refill {
    use super::*;

    const STEP: Duration = Duration::from_millis(10);

    /// Pacing 3 full packets per ms, 400 in the queue, window closed; 30 attempts in a stall, then the window opens.
    /// Result — the number of packets sent on opening; the allowance left after the closed attempts; how many full
    /// packets the bucket lets through at the first gate after opening (computed on a copy, with no time passing in the loop:
    /// a millisecond change in mid-loop replaces the allowance, and the count of sent ones depends on machine load).
    fn run(flag: bool) -> (usize, u32, u32) {
        let mut inner = stream_inner();
        let len = full_len(&inner) as u32;
        inner.congestion.pacing.set_rate(3 * len);
        inner.congestion.pacing.reset(Instant::now());
        inner.congestion.cwnd = 1000;
        inner.send_rwnd = 0;
        queue_full(&mut inner, 400);
        for _ in 0..30 {
            inner.congestion.pacing.rewind(STEP);
            assert!(inner.take_send_batch_with(flag).packets.is_empty());
        }
        let available = inner.congestion.pacing.available();
        inner.send_rwnd = u32::MAX;
        inner.congestion.pacing.rewind(STEP);
        let mut probe = inner.congestion.pacing.clone();
        probe.refill(Instant::now());
        let at_open = probe.available() / len;
        let b = inner.take_send_batch_with(flag);
        (b.packets.len(), available, at_open)
    }

    #[test]
    fn with_flag_the_stall_does_not_build_a_burst() {
        let (sent, _, at_open) = run(true);
        assert!(at_open <= 2 * 30, "gate lets through {at_open}");
        assert!(sent <= 2 * 30, "sent {sent}");
    }

    #[test]
    fn without_flag_the_stall_bursts() {
        let (_, available, at_open) = run(false);
        assert_eq!(
            available, 25_000,
            "a stall without the flag does not refill the bucket"
        );
        assert!(at_open >= 300, "gate lets through {at_open}");
    }

    #[test]
    fn closed_window_attempt_moves_last_refill_with_flag() {
        let mut inner = stream_inner();
        let len = full_len(&inner) as u32;
        inner.congestion.pacing.set_rate(3 * len);
        inner.congestion.pacing.reset(Instant::now());
        inner.send_rwnd = 0;
        queue_full(&mut inner, 5);
        inner.congestion.pacing.rewind(STEP);
        inner.take_send_batch_with(true);
        assert_eq!(inner.congestion.pacing.available(), 10 * 3 * len);
        // without the flag the allowance stayed the same (25 000)
        let mut off = stream_inner();
        off.congestion.pacing.set_rate(3 * len);
        off.congestion.pacing.reset(Instant::now());
        off.send_rwnd = 0;
        queue_full(&mut off, 5);
        off.congestion.pacing.rewind(STEP);
        off.take_send_batch_with(false);
        assert_eq!(off.congestion.pacing.available(), 25_000);
    }
}

mod unaddressed_head {
    use super::*;

    #[test]
    fn is_held_until_the_firewall_hook_gives_an_address() {
        // given: packets staged with the 0.0.0.0:0 stub (the firewall hook has not fired yet)
        let mut inner = stream_inner();
        queue_full(&mut inner, 2);
        inner
            .send_queue
            .readdress("127.0.0.1:9".parse().unwrap(), "0.0.0.0:0".parse().unwrap());
        // when: a wakeup before the address is known
        let batch = inner.take_send_batch();
        // then: nothing went to 0.0.0.0:0 (it would be lost and cost an RTO), the packets stay queued
        assert!(batch.packets.is_empty());
        assert_eq!(inner.send_queue.len(), 2);
        assert!(inner.outgoing.is_empty());
        // when: the hook fires and readdresses the queue
        inner
            .send_queue
            .readdress("0.0.0.0:0".parse().unwrap(), "127.0.0.1:9".parse().unwrap());
        // then: they go out
        assert_eq!(inner.take_send_batch().packets.len(), 2);
    }
}
