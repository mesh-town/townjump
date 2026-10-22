//! Tests of retransmissions through the gate (`retransmit.rs`): no burst in the ACK, retransmissions before new data, window and pacing gates.
use super::super::*;
use crate::native::stream::test_support::{stream_inner, with_queued, with_sent};
use std::time::Duration;

fn with_addr(inner: &mut StreamInner) {
    inner.remote_addr = Some("127.0.0.1:9".parse().unwrap());
}

/// `with_sent(0, count)` and an ACK with one SACK block `[count-1, count)`: the hole 0..count-1 is marked lost.
fn holes(count: u32) -> (StreamInner, bool) {
    let mut inner = stream_inner();
    with_addr(&mut inner);
    with_sent(&mut inner, 0, count);
    let block = SackRange {
        start: count - 1,
        end: count,
    };
    let outcome = inner.on_ack_packet(0, DEFAULT_RWND, &[block]);
    (inner, outcome.kick_pacing)
}

mod ack_with_holes {
    use super::*;

    #[test]
    fn sack_marks_holes_but_sends_nothing_outside_the_gates() {
        // given: 12 packets
        // when: SACK only for the last
        let (inner, kick) = holes(12);
        // then: holes marked, the retransmission did not go out in a burst, the sender is woken
        assert_eq!(inner.outgoing.resent(), 0);
        assert!(kick);
    }
}

mod kicked_step {
    use super::*;

    #[test]
    fn kicked_step_sends_cwnd_retransmits() {
        // given: 11 lost, the window is smaller
        let (mut inner, _) = holes(12);
        inner.congestion.cwnd = 4;
        let want = inner.congestion.cwnd.saturating_sub(inner.inflight()) as usize;
        // when
        let batch = inner.take_send_batch();
        // then: exactly until the window is full, we wait for an ACK, not pacing
        assert_eq!(batch.packets.len(), want.min(11));
        assert!(batch.pacing_delay.is_none());
        assert_eq!(inner.inflight(), 4);
    }
}

mod order {
    use super::*;

    #[test]
    fn retransmit_goes_before_queued_new_packet() {
        // given: 0..4 in flight, new seq 4 in the queue, SACK 1..4 — 0 is lost
        let mut inner = stream_inner();
        with_addr(&mut inner);
        with_sent(&mut inner, 0, 4);
        with_queued(&mut inner, 4);
        let block = SackRange { start: 1, end: 4 };
        inner.on_ack_packet(0, DEFAULT_RWND, &[block]);
        // when
        let batch = inner.take_send_batch();
        // then: first the head retransmission, then the new one
        assert_eq!(batch.packets.len(), 2);
        assert_eq!(batch.packets[0].0, inner.outgoing[&Seq(0)].packet);
    }
}

mod gates {
    use super::*;

    #[test]
    fn no_retransmit_while_cwnd_is_full() {
        // given: 0 lost, window equals flight, a new one in the queue
        let (mut inner, _) = holes(4);
        with_queued(&mut inner, 4);
        inner.congestion.cwnd = inner.inflight();
        // when
        let batch = inner.take_send_batch();
        // then: nothing went out, we wait for an ACK, not pacing
        assert!(batch.packets.is_empty());
        assert!(batch.pacing_delay.is_none());
        assert_eq!(inner.send_queue.len(), 1);
    }

    #[test]
    fn pacing_stops_retransmits_with_delay() {
        // given: there are lost ones, pacing 1 B/ms, the allowance is exhausted
        let (mut inner, _) = holes(4);
        inner.congestion.cwnd = 1000;
        inner.congestion.pacing.set_rate(1);
        inner
            .congestion
            .pacing
            .try_consume(u32::MAX, Instant::now());
        // when
        let batch = inner.take_send_batch();
        // then: the batch is empty, the delay is named
        assert!(batch.packets.is_empty());
        assert!(batch.pacing_delay.is_some());
    }

    #[test]
    fn rwnd_zero_does_not_hold_a_retransmit() {
        // given: the receiver window is closed
        let (mut inner, _) = holes(4);
        inner.congestion.cwnd = 1000;
        inner.send_rwnd = 0;
        // when
        let batch = inner.take_send_batch();
        // then: the retransmission went out (C 640-643)
        assert_eq!(batch.packets.len(), 3);
    }
}

mod queue {
    use super::*;

    #[test]
    fn first_lost_walks_in_seq_order_after_cursor() {
        // given: 0, 1, 2 lost
        let (inner, _) = holes(4);
        // then: walk in ascending order after the cursor
        assert_eq!(inner.outgoing.first_lost(None), Some(Seq(0)));
        assert_eq!(inner.outgoing.first_lost(Some(Seq(0))), Some(Seq(1)));
        assert_eq!(inner.outgoing.first_lost(Some(Seq(1))), Some(Seq(2)));
        assert_eq!(inner.outgoing.first_lost(Some(Seq(2))), None);
    }

    #[test]
    fn resend_clears_loss_and_counts() {
        // given: 0, 1, 2 lost; only 3 in flight
        let (mut inner, _) = holes(4);
        let (lost, resent, inflight) = (
            inner.outgoing.lost(),
            inner.outgoing.resent(),
            inner.inflight(),
        );
        // when
        assert!(inner.resend_seq(Seq(0), Instant::now()).is_some());
        // then
        assert_eq!(inner.outgoing.lost(), lost - 1);
        assert_eq!(inner.outgoing.resent(), resent + 1);
        assert_eq!(inner.inflight(), inflight + 1);
        assert!(inner.resend_seq(Seq(0), Instant::now()).is_none());
    }

    #[test]
    fn resend_takes_a_fresh_rate_info() {
        // given: 0, 1, 2 lost; since the first send `delivered` grew from 0 to 7
        let (mut inner, _) = holes(4);
        let first = inner.outgoing[&Seq(0)].rate_info.unwrap();
        inner.congestion.delivered = 7;
        let before = Instant::now();
        // when
        inner.resend_seq(Seq(0), before).unwrap();
        // then: like C `_send_packet` -> `udx__rate_pkt_sent`: a snapshot at the moment of retransmission
        let info = inner.outgoing[&Seq(0)].rate_info.unwrap();
        assert_eq!((first.delivered, info.delivered), (0, 7));
        assert!(info.time_sent >= before);
        assert_eq!(Some(info.first_sent_ts), inner.congestion.first_sent_ts);
    }

    #[test]
    fn resent_probe_counts_a_failure() {
        // given: an MTU probe among the sent ones, lost
        let mut inner = stream_inner();
        let base = Instant::now() - Duration::from_micros(10);
        for seq in 0..3u32 {
            let packet = build_data_packet(inner.remote_id, seq, 0, b"x");
            inner.register_sent(seq, packet, seq == 0);
            let at = base + Duration::from_micros(u64::from(seq));
            inner.outgoing.set_time_sent(Seq(seq), at);
        }
        inner.next_seq = 3;
        inner.on_sack(&[SackRange { start: 2, end: 3 }]);
        inner.detect_losses();
        let before = inner.mtu_probe_count;
        // when
        inner.resend_seq(Seq(0), Instant::now());
        // then
        assert_eq!(inner.mtu_probe_count, before + 1);
    }
}

mod route {
    use super::*;

    fn lost_one_with_route(route: Option<SocketAddr>) -> StreamInner {
        let mut inner = stream_inner();
        with_addr(&mut inner);
        with_sent(&mut inner, 0, 4);
        inner.outgoing.set_route(Seq(0), route);
        let block = SackRange { start: 1, end: 4 };
        inner.on_ack_packet(0, DEFAULT_RWND, &[block]);
        inner
    }

    #[test]
    fn retransmit_without_route_goes_to_current_remote_addr() {
        // given: 0 is lost, its route is None
        let mut inner = lost_one_with_route(None);
        // when
        let batch = inner.take_send_batch();
        // then: the retransmission goes to inner.remote_addr
        assert_eq!(batch.packets.len(), 1);
        assert_eq!(Some(batch.packets[0].1), inner.remote_addr);
    }

    #[test]
    fn retransmit_of_stamped_packet_goes_to_its_old_address() {
        // given: 0 is lost and was sent to an old address; the remote has moved since
        let old: SocketAddr = "127.0.0.1:1111".parse().unwrap();
        let mut inner = lost_one_with_route(Some(old));
        // when
        let batch = inner.take_send_batch();
        // then: the retransmission goes to the old address, not to the current one
        assert_eq!(batch.packets.len(), 1);
        assert_eq!(batch.packets[0].1, old);
        assert_ne!(Some(old), inner.remote_addr);
    }
}
