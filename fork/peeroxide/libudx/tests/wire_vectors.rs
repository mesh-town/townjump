//! The wire against JS libudx vectors (`testvectors/deltas/udx.json`, `results.new`, udx-native 1.21.3):
//! a raw UDP peer sends packets to a Rust stream and compares the replies byte for byte.
//! Vector tails are constants with a jq path; the test does not read the repository file itself.
#![deny(clippy::all)]

mod common;

use std::collections::BTreeSet;
use std::time::Duration;

use common::raw_peer::{
    FLAG_DATA, FLAG_HEARTBEAT, Link, OPEN_WINDOW, RawPeer, ack_of, ack_packet, data_packet,
    end_packet, flags_of, from_hex, hex, seq_of,
};
use common::with_timeout;

const REPLY_WAIT: Duration = Duration::from_secs(1);
const TEST_LIMIT: Duration = Duration::from_secs(20);

// jq: .heartbeatReply[] | .sent, .replies[0].hex
const HEARTBEAT_PLAIN: &str = "ff01200005000000000040000000000000000000";
const HEARTBEAT_GARBAGE: &str = "ff0120000500000000000000e703000039300000";
const HEARTBEAT_REPLY: &str = "ff01000007000000000040000000000000000000";

// jq: .ackPerData.steps[] | [.sent, .replies[0].hex]; steps go in send order (seq 2 is repeated at the end)
#[rustfmt::skip]
const ACK_PER_DATA: [(&str, &str); 9] = [
    ("ff01010005000000000040000000000000000000a0", "ff01000007000000000040000000000001000000"),
    ("ff01010005000000000040000100000000000000a1", "ff01000007000000000040000000000002000000"),
    ("ff01010005000000000040000200000000000000a2", "ff01000007000000000040000000000003000000"),
    ("ff01010005000000000040000400000000000000a4", "ff010400070000000000400000000000030000000400000005000000"),
    ("ff01010005000000000040000500000000000000a5", "ff010400070000000000400000000000030000000400000006000000"),
    ("ff01010005000000000040000700000000000000a7", "ff0104000700000000004000000000000300000004000000060000000700000008000000"),
    ("ff01010005000000000040000300000000000000a3", "ff010400070000000000400000000000060000000700000008000000"),
    ("ff01010005000000000040000600000000000000a6", "ff01000007000000000040000000000008000000"),
    ("ff01010005000000000040000200000000000000a2", "ff01000007000000000040000000000008000000"),
];
// jq: .ackPerData.deliveredInOrder — a0..a7
const DELIVERED_IN_ORDER: [u8; 8] = [0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7];

/// How many DATA the peer expects from the stream in the `strictSack_*` scenarios (jq: `.firstDataSeqs`).
const FIRST_DATA_PACKETS: u32 = 6;
/// Payload of one packet (MTU 1200 minus IPv4 overhead 48).
const PAYLOAD_PER_PACKET: usize = 1152;

mod heartbeat_reply {
    use super::*;

    mod plain_heartbeat {
        use super::*;

        #[tokio::test]
        async fn is_answered_with_plain_ack_byte_for_byte() {
            with_timeout(TEST_LIMIT, async {
                let link = Link::new().await;

                let reply = when_sent(&link.peer, HEARTBEAT_PLAIN).await;

                assert_eq!(reply, HEARTBEAT_REPLY);
            })
            .await;
        }
    }

    mod heartbeat_with_garbage_fields {
        use super::*;

        #[tokio::test]
        async fn is_answered_with_the_same_plain_ack() {
            with_timeout(TEST_LIMIT, async {
                let link = Link::new().await;

                let reply = when_sent(&link.peer, HEARTBEAT_GARBAGE).await;

                assert_eq!(reply, HEARTBEAT_REPLY);
            })
            .await;
        }

        #[tokio::test]
        async fn does_not_close_the_send_window() {
            with_timeout(TEST_LIMIT, async {
                // given: a garbage HEARTBEAT with window 0 already got a reply
                let Link {
                    peer,
                    stream,
                    keep_alive: _keep,
                } = Link::new().await;
                when_sent(&peer, HEARTBEAT_GARBAGE).await;
                // when
                let _write = tokio::spawn(async move { stream.write(b"after garbage").await });
                // then: the window from the garbage is not read, DATA goes out at once
                let (_, packet) = peer
                    .recv_within(Duration::from_millis(200))
                    .await
                    .expect("DATA did not arrive within 200 ms: window 0 from HEARTBEAT was read");
                assert_ne!(
                    flags_of(&packet) & FLAG_DATA,
                    0,
                    "type_flags = {packet:02x?}"
                );
            })
            .await;
        }
    }
}

mod ack_per_data {
    use super::*;

    #[tokio::test]
    async fn every_reply_matches_js_bytes() {
        with_timeout(TEST_LIMIT, async {
            let link = Link::new().await;

            for (step, (sent, expected)) in ACK_PER_DATA.iter().enumerate() {
                let reply = when_sent(&link.peer, sent).await;

                assert_eq!(&reply, expected, "step {step}, sent {sent}");
            }
        })
        .await;
    }

    #[tokio::test]
    async fn read_delivers_payloads_in_order_without_the_duplicate() {
        with_timeout(TEST_LIMIT, async {
            // given: the peer sent all 9 packets of the vector (seq 2 twice, seq 3 and 6 later)
            let mut link = Link::new().await;
            for (sent, _) in ACK_PER_DATA {
                when_sent(&link.peer, sent).await;
            }
            // when
            let delivered = read_bytes(&mut link, DELIVERED_IN_ORDER.len()).await;
            // then
            assert_eq!(delivered, DELIVERED_IN_ORDER);
            let extra = tokio::time::timeout(Duration::from_millis(100), link.stream.read()).await;
            assert!(
                extra.is_err(),
                "duplicate seq 2 must not be read: {extra:?}"
            );
        })
        .await;
    }
}

mod duplicate_data {
    use super::*;

    #[tokio::test]
    async fn is_acknowledged_again() {
        with_timeout(TEST_LIMIT, async {
            // given: seq 0 received and acked
            let link = Link::new().await;
            when_data_sent(&link.peer, 0).await;
            // when: the same packet arrived again
            let (_, ack) = when_data_sent(&link.peer, 0).await;
            // then: there is an ACK for the duplicate (D8) and the cumulative ack is unchanged
            assert_eq!(ack_of(&ack), 1, "reply = {}", hex(&ack));
        })
        .await;
    }
}

mod stream_end {
    use super::*;

    #[tokio::test]
    async fn is_acknowledged_with_ack_past_end_seq() {
        with_timeout(TEST_LIMIT, async {
            // given: seq 0 received
            let link = Link::new().await;
            when_data_sent(&link.peer, 0).await;
            // when: END with seq 1
            link.peer.send(&end_packet(1)).await;
            // then
            let (_, reply) = link
                .peer
                .recv_within(REPLY_WAIT)
                .await
                .expect("ACK for END");
            assert_eq!(ack_of(&reply), 2, "reply = {}", hex(&reply));
        })
        .await;
    }
}

mod idle_stream {
    use super::*;

    #[tokio::test]
    async fn sends_no_heartbeat_for_2_5_seconds() {
        with_timeout(TEST_LIMIT, async {
            // given: the exchange is over, the streams are idle
            let link = Link::new().await;
            when_data_sent(&link.peer, 0).await;
            // when
            let packets = link.peer.collect_for(Duration::from_millis(2500)).await;
            // then: no one-second keepalive
            let heartbeats: Vec<_> = packets
                .iter()
                .filter(|(_, p)| flags_of(p) & FLAG_HEARTBEAT != 0)
                .collect();
            assert!(heartbeats.is_empty(), "extra HEARTBEATs: {heartbeats:?}");
        })
        .await;
    }
}

/// The stream sends 6 DATA, the peer answers with one ACK per the vector plan; checks which seqs are retransmitted within 1500 ms.
mod strict_sack {
    use super::*;

    mod block_inside_sent_range {
        use super::*;

        #[tokio::test]
        async fn retransmits_packets_behind_the_block() {
            // jq: .strictSack_valid — ack 0, [0,2); JS retransmitted [5]
            let retransmitted = retransmitted_after(0, &[(0, 2)]).await;

            // rs has no TLP: seq 5 in JS is a TLP, subtracted from the JS expectation; within 1500 ms RTO marks everything, cwnd = 1, one retransmission goes out
            then_retransmitted(&retransmitted, &[2], &[]);
        }
    }

    mod block_ending_at_next_seq {
        use super::*;

        #[tokio::test]
        async fn retransmits_nothing() {
            // jq: .strictSack_endEqSeq — ack 0, [0,6) at seq 6; JS: []
            let retransmitted = retransmitted_after(0, &[(0, 6)]).await;

            then_retransmitted(&retransmitted, &[], &[]);
        }
    }

    mod block_ending_above_next_seq {
        use super::*;

        #[tokio::test]
        async fn is_ignored_and_everything_is_retransmitted() {
            // jq: .strictSack_endAboveSeq — ack 0, [0,7); JS: [0]
            let retransmitted = retransmitted_after(0, &[(0, 7)]).await;

            then_retransmitted(&retransmitted, &[0], &[0]);
        }
    }

    mod block_starting_below_acked {
        use super::*;

        #[tokio::test]
        async fn is_ignored_and_unacked_packets_are_retransmitted() {
            // jq: .strictSack_startBelowAcked — ack 2, [1,3); JS: [2,5]
            let retransmitted = retransmitted_after(2, &[(1, 3)]).await;

            // rs has no TLP: seq 5 in JS is a TLP, subtracted from the JS expectation; within 1500 ms RTO marks everything, cwnd = 1, one retransmission goes out
            then_retransmitted(&retransmitted, &[2], &[2]);
        }
    }

    mod block_starting_at_acked {
        use super::*;

        #[tokio::test]
        async fn marks_its_packets_and_retransmits_the_rest() {
            // jq: .strictSack_startAtAcked — ack 2, [2,4); JS: [4]
            let retransmitted = retransmitted_after(2, &[(2, 4)]).await;

            then_retransmitted(&retransmitted, &[4], &[4]);
        }
    }

    mod empty_block_before_valid_one {
        use super::*;

        #[tokio::test]
        async fn is_skipped_and_the_valid_block_still_works() {
            // jq: .strictSack_startEqEnd — ack 0, [0,0) then [0,2); JS: [5]
            let retransmitted = retransmitted_after(0, &[(0, 0), (0, 2)]).await;

            // rs has no TLP: seq 5 in JS is a TLP, subtracted from the JS expectation; within 1500 ms RTO marks everything, cwnd = 1, one retransmission goes out
            then_retransmitted(&retransmitted, &[2], &[]);
        }
    }

    mod block_with_end_below_start_before_valid_one {
        use super::*;

        #[tokio::test]
        async fn is_skipped_and_the_valid_block_still_works() {
            // jq: .strictSack_endBelowStart — ack 0, [5,0) then [0,2); JS: [5]
            let retransmitted = retransmitted_after(0, &[(5, 0), (0, 2)]).await;

            // rs has no TLP: seq 5 in JS is a TLP, subtracted from the JS expectation; within 1500 ms RTO marks everything, cwnd = 1, one retransmission goes out
            then_retransmitted(&retransmitted, &[2], &[]);
        }
    }

    mod valid_and_invalid_blocks_in_one_packet {
        use super::*;

        #[tokio::test]
        async fn only_the_valid_block_counts() {
            // jq: .strictSack_mixed — ack 0, [0,2) then [4,9); JS: [5]
            let retransmitted = retransmitted_after(0, &[(0, 2), (4, 9)]).await;

            // rs has no TLP: seq 5 in JS is a TLP, subtracted from the JS expectation; within 1500 ms RTO marks everything, cwnd = 1, one retransmission goes out
            then_retransmitted(&retransmitted, &[2], &[]);
        }
    }
}

// ── setup and helpers ────────────────────────────────────────────────────────

/// The peer sends a packet from hex and returns the reply hex (up to 1 s).
async fn when_sent(peer: &RawPeer, sent_hex: &str) -> String {
    peer.send(&from_hex(sent_hex)).await;
    let (_, reply) = peer
        .recv_within(REPLY_WAIT)
        .await
        .unwrap_or_else(|| panic!("no reply to {sent_hex} within {REPLY_WAIT:?}"));
    hex(&reply)
}

/// The peer sends DATA with `seq` and a one-byte payload; returns the stream's reply.
async fn when_data_sent(peer: &RawPeer, seq: u32) -> (u64, Vec<u8>) {
    peer.send(&data_packet(seq, &[0xa0 + seq as u8])).await;
    peer.recv_within(REPLY_WAIT)
        .await
        .unwrap_or_else(|| panic!("no reply to DATA seq {seq} within {REPLY_WAIT:?}"))
}

/// Reads from the stream until `count` bytes are collected (each read up to 2 s).
async fn read_bytes(link: &mut Link, count: usize) -> Vec<u8> {
    let mut bytes = Vec::new();
    while bytes.len() < count {
        let read = tokio::time::timeout(Duration::from_secs(2), link.stream.read());
        let chunk = read.await.expect("read did not return within 2 s");
        bytes.extend(
            chunk
                .expect("read failed")
                .expect("EOF before the end of data"),
        );
    }
    bytes
}

/// The stream writes 6 DATA; the peer answers with one ACK (`ack`, `blocks`) and collects 1500 ms of retransmissions.
async fn retransmitted_after(ack: u32, blocks: &[(u32, u32)]) -> BTreeSet<u32> {
    // given: the stream sent DATA seq 0..5, the peer received them
    let Link {
        mut peer,
        stream,
        keep_alive: _keep,
    } = Link::new().await;
    let payload = vec![7u8; PAYLOAD_PER_PACKET * FIRST_DATA_PACKETS as usize];
    let _write = tokio::spawn(async move { stream.write(&payload).await });
    let first: Vec<u32> = recv_data_seqs(&peer, FIRST_DATA_PACKETS).await;
    assert_eq!(
        first,
        (0..FIRST_DATA_PACKETS).collect::<Vec<_>>(),
        "first DATA"
    );
    // when
    peer.restart_clock();
    peer.send(&ack_packet(ack, OPEN_WINDOW, blocks)).await;
    // then: collect the retransmissions
    let packets = peer.collect_for(Duration::from_millis(1500)).await;
    packets
        .iter()
        .filter(|(_, p)| flags_of(p) & FLAG_DATA != 0)
        .map(|(_, p)| seq_of(p))
        .collect()
}

/// The seq numbers of the first `count` DATA packets that reached the peer.
async fn recv_data_seqs(peer: &RawPeer, count: u32) -> Vec<u32> {
    let mut seqs = Vec::new();
    while seqs.len() < count as usize {
        let (_, packet) = peer
            .recv_within(Duration::from_secs(2))
            .await
            .expect("DATA did not arrive within 2 s");
        if flags_of(&packet) & FLAG_DATA != 0 {
            seqs.push(seq_of(&packet));
        }
    }
    seqs
}

/// The set of retransmissions equals the Rust expectation and includes everything JS retransmitted.
fn then_retransmitted(actual: &BTreeSet<u32>, rust: &[u32], js: &[u32]) {
    let rust: BTreeSet<u32> = rust.iter().copied().collect();
    let js: BTreeSet<u32> = js.iter().copied().collect();
    assert_eq!(actual, &rust, "retransmitted seqs within 1500 ms");
    assert!(
        js.is_subset(actual),
        "JS retransmitted {js:?}, Rust — {actual:?}"
    );
}
