//! Packet number wrap across 2^32: send, ACK, losses, receive, SACK blocks (JS: uint32 in udx.c).
use super::*;
use crate::native::stream::test_support::{
    resend_lost_all, rto_resend, stream_inner, with_pending_write, with_sent,
};
use std::sync::Arc;
use tokio::net::UdpSocket;

const MAX: u32 = u32::MAX;

mod sending {
    use super::*;

    #[tokio::test]
    async fn write_over_the_wrap_leaves_next_seq_at_zero() {
        // given: next number MAX-2, a write of three packets
        let mut inner = connected_stream(MAX - 2).await;
        let size = 3 * inner.max_payload();
        // when
        let prep = inner.prepare_write(size).unwrap();
        // then: MAX-2, MAX-1, MAX taken; the write threshold and next_seq — 0
        assert_eq!((prep.first_seq, inner.next_seq), (MAX - 2, 0));
        assert_eq!(inner.pending_writes[0].ack_threshold, 0);
    }

    #[tokio::test]
    async fn end_at_max_takes_the_last_number_and_wraps() {
        // given: next number MAX
        let mut inner = connected_stream(MAX).await;
        // when
        let prep = inner.prepare_end().unwrap();
        // then
        assert_eq!((prep.first_seq, inner.next_seq), (MAX, 0));
        assert_eq!(inner.pending_writes[0].ack_threshold, 0);
    }
}

mod ordering {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn seq_keys_sort_across_the_wrap() {
        // given: numbers shuffled
        let keys: BTreeSet<Seq> = [1, MAX, 0, MAX - 1].map(Seq).into();
        // when
        let order: Vec<u32> = keys.iter().map(|seq| seq.0).collect();
        // then: MAX-1 < MAX < 0 < 1
        assert_eq!(order, vec![MAX - 1, MAX, 0, 1]);
    }
}

mod end_across_the_wrap {
    use super::*;

    #[tokio::test]
    async fn ack_zero_confirms_an_end_taken_at_max() {
        // given: END with number MAX, next_seq = 0
        let mut inner = connected_stream(MAX).await;
        inner.prepare_end().unwrap();
        assert_eq!(inner.end_seq, Some(MAX));
        // when: ack 0 stepped over MAX
        let outcome = inner.on_ack_packet(0, DEFAULT_RWND, &[]);
        // then: exactly one ack
        assert!(outcome.end_acked);
        assert!(!inner.on_ack_packet(0, DEFAULT_RWND, &[]).end_acked);
    }
}

mod cumulative_ack {
    use super::*;

    #[test]
    fn across_the_wrap_removes_only_acked_packets() {
        // given: MAX-2, MAX-1, MAX, 0, 1 in flight
        let mut inner = sent_stream(MAX - 2, 5);
        // when: ack 0 acks the first three
        let (_, rate) = inner.on_cumulative_ack(0);
        // then
        assert_eq!(outgoing_seqs(&inner), vec![0, 1]);
        assert_eq!(inner.remote_acked, 0);
        let acked: Vec<u32> = rate.iter().map(|(seq, _)| *seq).collect();
        assert_eq!(acked, vec![MAX - 2, MAX - 1, MAX]);
    }

    #[test]
    fn completes_a_write_whose_threshold_is_before_the_wrap() {
        // given: a write with threshold MAX, MAX-2..1 in flight
        let mut inner = sent_stream(MAX - 2, 5);
        let _write = with_pending_write(&mut inner, MAX);
        // when: ack 1 — already past the wrap
        let (completed, _) = inner.on_cumulative_ack(1);
        // then
        assert_eq!(completed.len(), 1);
    }

    #[test]
    fn keeps_a_write_whose_threshold_is_after_the_wrap() {
        // given: a write with threshold 1, MAX-2..1 in flight
        let mut inner = sent_stream(MAX - 2, 5);
        let _write = with_pending_write(&mut inner, 1);
        // when: ack 0 has not reached the threshold yet
        let (completed, _) = inner.on_cumulative_ack(0);
        // then
        assert!(completed.is_empty());
    }

    #[test]
    fn ack_zero_after_the_wrap_is_a_real_ack() {
        // given: MAX-2, MAX-1, MAX in flight; next_seq = 0
        let mut inner = sent_stream(MAX - 2, 3);
        // when: ack 0 (not an "empty" ACK but the ack of MAX)
        let outcome = inner.on_ack_packet(0, DEFAULT_RWND, &[]);
        // then
        assert_eq!(outgoing_seqs(&inner), Vec::<u32>::new());
        assert_eq!(inner.remote_acked, 0);
        assert_eq!(outcome.rto_timer, Some(FAR_FUTURE));
    }
}

mod retransmission {
    use super::*;

    #[test]
    fn timer_retransmit_goes_in_seq_order_across_the_wrap() {
        // given: MAX-1, MAX, 0, 1 in flight
        let mut inner = sent_stream(MAX - 1, 4);
        // when
        let packets = rto_resend(&mut inner);
        // then
        assert_eq!(packet_seqs(&packets), vec![MAX - 1, MAX, 0, 1]);
    }

    #[test]
    fn fast_retransmit_goes_in_seq_order_across_the_wrap() {
        // given: MAX-2..2 in flight, 1 and 2 received by SACK
        let mut inner = sent_stream(MAX - 2, 6);
        inner.on_sack(&[SackRange { start: 1, end: 3 }]);
        // when
        let lost = inner.detect_losses();
        let packets = resend_lost_all(&mut inner);
        // then: all four before 1 are lost, retransmission in ascending order
        assert_eq!(lost, 4);
        assert_eq!(packet_seqs(&packets), vec![MAX - 2, MAX - 1, MAX, 0]);
    }

    #[test]
    fn recovery_boundary_is_the_last_sent_number() {
        // given: the same SACK; entering Recovery records the boundary
        let mut inner = sent_stream(MAX - 2, 6);
        inner.on_sack(&[SackRange { start: 1, end: 3 }]);
        // when
        inner.detect_losses();
        // then: the boundary is 2 (the last sent), not MAX (the numeric maximum)
        assert_eq!(inner.congestion.high_seq, 2);
    }
}

mod receiving {
    use super::*;

    #[test]
    fn in_order_packets_cross_the_wrap() {
        // given: waiting for MAX-1
        let mut inner = stream_inner();
        inner.next_remote_seq = MAX - 1;
        // when: MAX-1, MAX, 0, 1 in a row
        for seq in [MAX - 1, MAX, 0, 1] {
            let events = inner.receive_data(seq, vec![1], FLAG_DATA);
            assert_eq!(payloads(events).len(), 1, "seq {seq} delivered at once");
        }
        // then
        assert_eq!(inner.next_remote_seq, 2);
    }

    #[test]
    fn early_packet_after_the_wrap_waits_in_the_buffer() {
        // given: waiting for MAX-1; 0 and MAX arrived
        let mut inner = stream_inner();
        inner.next_remote_seq = MAX - 1;
        let early = [(0, b"c"), (MAX, b"b")];
        for (seq, byte) in early {
            let events = inner.receive_data(seq, byte.to_vec(), FLAG_DATA);
            assert!(payloads(events).is_empty(), "seq {seq} into the buffer");
        }
        // when: MAX-1 arrived
        let events = inner.receive_data(MAX - 1, b"a".to_vec(), FLAG_DATA);
        // then: in-order delivery, the buffer is empty
        assert_eq!(
            payloads(events),
            vec![b"a".to_vec(), b"b".into(), b"c".into()]
        );
        assert_eq!(inner.next_remote_seq, 1);
        assert!(inner.recv_buf.is_empty());
    }

    #[test]
    fn packet_behind_the_wrap_is_a_duplicate() {
        // given: received up to and including 0, waiting for 1
        let mut inner = stream_inner();
        inner.next_remote_seq = 1;
        // when: an old MAX arrived
        let events = inner.receive_data(MAX, vec![1], FLAG_DATA);
        // then: not delivered and not buffered
        assert!(payloads(events).is_empty());
        assert!(inner.recv_buf.is_empty());
    }

    #[test]
    fn sack_blocks_cross_the_wrap() {
        // given: waiting for MAX-1; MAX, 0, 1 and 3 in the buffer
        let mut inner = stream_inner();
        inner.next_remote_seq = MAX - 1;
        for seq in [3, 1, 0, MAX] {
            buffer_out_of_order(&mut inner, seq);
        }
        // when
        let blocks = inner.sack_ranges();
        // then: [MAX, 2) and [3, 4)
        let expected = [(MAX, 2), (3, 4)].map(|(start, end)| SackRange { start, end });
        assert_eq!(blocks, expected);
    }
}

// ── setup and helpers ────────────────────────────────────────────────────────

/// A connected stream with a real socket: `next_seq` and `remote_acked` = `next_seq`.
async fn connected_stream(next_seq: u32) -> StreamInner {
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let mut inner = stream_inner();
    inner.connected = true;
    inner.remote_id = 7;
    inner.remote_addr = Some("127.0.0.1:9".parse().unwrap());
    inner.udp = Some(Arc::new(socket));
    inner.next_seq = next_seq;
    inner.remote_acked = next_seq;
    inner
}

/// A stream with `count` sent DATA, starting at `first` (mod 2^32).
fn sent_stream(first: u32, count: u32) -> StreamInner {
    let mut inner = stream_inner();
    with_sent(&mut inner, first, count);
    inner
}

/// `outgoing` numbers in map key order.
fn outgoing_seqs(inner: &StreamInner) -> Vec<u32> {
    inner.outgoing.keys().map(|seq| seq.0).collect()
}

/// Puts a packet without payload and flags into the receive buffer.
fn buffer_out_of_order(inner: &mut StreamInner, seq: u32) {
    let packet = RecvdPacket {
        payload: Vec::new(),
        flags: 0,
    };
    inner.recv_buf.insert(Seq(seq), packet);
}

fn packet_seqs(packets: &[Vec<u8>]) -> Vec<u32> {
    let seq_of = |packet: &Vec<u8>| Header::decode(packet).unwrap().seq;
    packets.iter().map(seq_of).collect()
}

/// Payloads of `Data` events in order (`End` skipped).
fn payloads(events: Vec<StreamEvent>) -> Vec<Vec<u8>> {
    let data = |event| match event {
        StreamEvent::Data(bytes) => Some(bytes),
        StreamEvent::End => None,
    };
    events.into_iter().filter_map(data).collect()
}
