//! ACK and HEARTBEAT packet tests: bytes as in JS libudx, SACK block limit, `seq` in replies.
use super::*;
use crate::native::header::{
    FLAG_HEARTBEAT, FLAG_SACK, HEADER_SIZE, Header, SackRange, decode_sack,
};
use crate::native::stream::test_support::{stream_inner, with_queued};
use crate::native::stream::{RecvdPacket, StreamInner};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;

// Vector tails of testvectors/deltas/udx.json (`results.new`), remote_id = 7.
// jq: .ackPerData.steps[0].replies[0].hex — reply to seq 0, ack 1, no blocks
const ACK_PLAIN: &str = "ff01000007000000000040000000000001000000";
// jq: .ackPerData.steps[3].replies[0].hex — reply to seq 4, ack 3, block [4,5)
const ACK_BLOCK_4_5: &str = "ff010400070000000000400000000000030000000400000005000000";
// jq: .ackPerData.steps[5].replies[0].hex — reply to seq 7, ack 3, blocks [4,6) and [7,8)
const ACK_BLOCKS_4_6_7_8: &str =
    "ff0104000700000000004000000000000300000004000000060000000700000008000000";
// jq: .packetsOnWire.keepalive300.firstHeartbeat.hex
const HEARTBEAT: &str = "ff01200007000000000040000000000000000000";

/// 20 header bytes + 50 blocks of 8 bytes.
const MAX_ACK_LEN: usize = 420;

mod ack_packet {
    use super::*;

    mod without_blocks {
        use super::*;

        #[test]
        fn is_plain_20_byte_ack() {
            let packet = ack_packet(7, 0, 1, &[]);

            assert_eq!(hex(&packet), ACK_PLAIN);
        }
    }

    mod one_block {
        use super::*;

        #[test]
        fn carries_sack_flag_and_the_block() {
            let packet = ack_packet(7, 0, 3, &ranges(&[(4, 5)]));

            assert_eq!(hex(&packet), ACK_BLOCK_4_5);
        }
    }

    mod two_blocks {
        use super::*;

        #[test]
        fn carries_both_blocks_in_order() {
            let packet = ack_packet(7, 0, 3, &ranges(&[(4, 6), (7, 8)]));

            assert_eq!(hex(&packet), ACK_BLOCKS_4_6_7_8);
        }
    }

    mod fifty_blocks {
        use super::*;

        #[test]
        fn is_420_bytes_with_zero_data_offset() {
            // given: exactly the block limit
            let sacks = many_ranges(50);
            // when
            let packet = ack_packet(7, 0, 3, &sacks);
            // then
            assert_eq!(packet.len(), MAX_ACK_LEN);
            assert_eq!(packet[3], 0, "data_offset = 0, blocks follow the 20th byte");
            assert_eq!(packet[2], FLAG_SACK, "type_flags");
            assert_eq!(decode_sack(&packet[HEADER_SIZE..]).unwrap(), sacks);
        }
    }

    mod sixty_blocks {
        use super::*;

        #[test]
        fn are_cut_to_first_fifty() {
            // given: more blocks than the limit
            let sacks = many_ranges(60);
            // when
            let packet = ack_packet(7, 0, 3, &sacks);
            // then
            assert_eq!(packet.len(), MAX_ACK_LEN);
            assert_eq!(decode_sack(&packet[HEADER_SIZE..]).unwrap(), sacks[..50]);
        }
    }
}

mod heartbeat_packet {
    use super::*;

    #[test]
    fn matches_js_keepalive_bytes() {
        let packet = heartbeat_packet(7, 0, 0);

        assert_eq!(hex(&packet), HEARTBEAT);
    }

    #[test]
    fn carries_given_seq_and_ack_in_20_bytes() {
        // when
        let packet = heartbeat_packet(7, 5, 9);
        // then
        let header = Header::decode(&packet).unwrap();
        assert_eq!(packet.len(), HEADER_SIZE);
        assert_eq!(header.type_flags, FLAG_HEARTBEAT);
        assert_eq!((header.seq, header.ack), (5, 9));
    }
}

mod next_send_seq {
    use super::*;

    mod queue_is_empty {
        use super::*;

        #[test]
        fn is_next_seq() {
            // given
            let mut inner = stream_inner();
            inner.next_seq = 6;
            // then
            assert_eq!(inner.next_send_seq(), 6);
        }
    }

    mod packets_wait_in_queue {
        use super::*;

        #[test]
        fn is_seq_of_queue_head() {
            // given: an unsent seq 4 in the queue, next_seq = 6
            let mut inner = stream_inner();
            inner.next_seq = 6;
            with_queued(&mut inner, 4);
            // then
            assert_eq!(inner.next_send_seq(), 4);
        }

        #[test]
        fn ignores_packets_behind_the_head() {
            // given: seq 4 in the queue, 5 behind it
            let mut inner = stream_inner();
            inner.next_seq = 6;
            with_queued(&mut inner, 4);
            with_queued(&mut inner, 5);
            // then
            assert_eq!(inner.next_send_seq(), 4);
        }
    }
}

mod sending {
    use super::*;

    #[test]
    fn raw_packet_without_socket_is_dropped_silently() {
        // given: the stream has no socket; a plain #[test] without a runtime — spawn would fail
        let inner = stream_inner();
        // when
        send_raw(&inner, vec![1, 2, 3]);
        // then: we got here without a panic
    }

    #[tokio::test]
    async fn ack_carries_next_send_seq_and_next_remote_seq() {
        // given: receive reached seq 3, an unsent seq 4 in the queue
        let mut wired = wired_stream().await;
        wired.inner.next_remote_seq = 3;
        with_queued(&mut wired.inner, 4);
        // when
        send_ack(&wired.inner);
        // then
        let packet = wired.received().await;
        let header = Header::decode(&packet).unwrap();
        assert_eq!(packet.len(), HEADER_SIZE, "no blocks");
        assert_eq!((header.remote_id, header.seq, header.ack), (7, 4, 3));
    }

    #[tokio::test]
    async fn ack_carries_current_out_of_order_blocks() {
        // given: seq 5, 6 and 9 received with next_remote_seq = 3
        let mut wired = wired_stream().await;
        wired.inner.next_remote_seq = 3;
        for seq in [5, 6, 9] {
            wired.inner.recv_buf.insert(Seq(seq), out_of_order());
        }
        // when
        send_ack(&wired.inner);
        // then
        let packet = wired.received().await;
        let header = Header::decode(&packet).unwrap();
        assert!(header.has_flag(FLAG_SACK), "SACK flag");
        let blocks = decode_sack(&packet[HEADER_SIZE..]).unwrap();
        assert_eq!(blocks, ranges(&[(5, 7), (9, 10)]));
    }
}

// ── setup and helpers ────────────────────────────────────────────────────────

/// A stream with a real UDP socket and a peer that reads what was sent.
struct Wired {
    inner: StreamInner,
    peer: UdpSocket,
}

/// A stream with `remote_id = 7` that sends to the peer's socket.
async fn wired_stream() -> Wired {
    let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let own = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let mut inner = stream_inner();
    inner.remote_id = 7;
    inner.remote_addr = Some(peer.local_addr().unwrap());
    inner.udp = Some(Arc::new(own));
    Wired { inner, peer }
}

impl Wired {
    /// The first packet that reached the peer (within 1 s).
    async fn received(&self) -> Vec<u8> {
        let mut buf = [0u8; 2048];
        let read = tokio::time::timeout(Duration::from_secs(1), self.peer.recv_from(&mut buf));
        let (len, _) = read
            .await
            .expect("packet did not arrive within 1 s")
            .unwrap();
        buf[..len].to_vec()
    }
}

fn out_of_order() -> RecvdPacket {
    RecvdPacket {
        payload: Vec::new(),
        flags: 0,
    }
}

fn ranges(pairs: &[(u32, u32)]) -> Vec<SackRange> {
    pairs
        .iter()
        .map(|&(start, end)| SackRange { start, end })
        .collect()
}

/// `count` blocks `[10i, 10i+1)`: gaps between them, no merging.
fn many_ranges(count: u32) -> Vec<SackRange> {
    (0..count)
        .map(|i| SackRange {
            start: 10 * i,
            end: 10 * i + 1,
        })
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
