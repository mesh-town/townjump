//! D8/END against JS libudx (`tests/node/udx-end-ack.js`, scenarios E1–E3, udx-native 1.19.2 and 1.21.3):
//! the ACK for the ack of our END goes out once and only if the peer has not finished itself.
#![deny(clippy::all)]

mod common;

use std::time::Duration;

use common::raw_peer::{Link, OPEN_WINDOW, RawPeer, ack_packet, end_packet, hex};
use common::with_timeout;

const REPLY_WAIT: Duration = Duration::from_secs(1);
const QUIET: Duration = Duration::from_millis(500);
const TEST_LIMIT: Duration = Duration::from_secs(20);

// Vectors of `udx-end-ack.js`: Rust stream with id 5, peer with id 7, window 0x400000.
const OUR_END_AT_0: &str = "ff01020007000000000040000000000000000000";
const FINAL_ACK: &str = "ff01000007000000000040000100000000000000";
const ACK_OF_PEER_END: &str = "ff01000007000000000040000000000001000000";
const OUR_END_AFTER_PEER_END: &str = "ff01020007000000000040000000000001000000";

mod we_end_first {
    use super::*;

    #[tokio::test]
    async fn acked_end_is_answered_with_one_final_ack() {
        with_timeout(TEST_LIMIT, async {
            // given: the stream closes the write side first, the peer got END
            let link = Link::new().await;
            let scenario = async {
                expect_packet(&link.peer, OUR_END_AT_0, "stream END").await;
                // when: the peer acks END with a pure ACK
                link.peer.send(&ack_packet(1, OPEN_WINDOW, &[])).await;
                // then: exactly one final ACK, then silence
                expect_packet(&link.peer, FINAL_ACK, "final ACK").await;
                expect_silence(&link.peer).await;
            };
            let (shutdown, ()) = tokio::join!(link.stream.shutdown(), scenario);
            shutdown.expect("shutdown after ACK");
        })
        .await;
    }

    #[tokio::test]
    async fn repeated_ack_of_the_same_end_gets_no_reply() {
        with_timeout(TEST_LIMIT, async {
            // given: END is acked and the final ACK received
            let link = Link::new().await;
            let scenario = async {
                expect_packet(&link.peer, OUR_END_AT_0, "stream END").await;
                link.peer.send(&ack_packet(1, OPEN_WINDOW, &[])).await;
                expect_packet(&link.peer, FINAL_ACK, "final ACK").await;
                // when: the same ACK once more
                link.peer.send(&ack_packet(1, OPEN_WINDOW, &[])).await;
                // then
                expect_silence(&link.peer).await;
            };
            let (shutdown, ()) = tokio::join!(link.stream.shutdown(), scenario);
            shutdown.expect("shutdown after ACK");
        })
        .await;
    }
}

mod peer_ends_first {
    use super::*;

    #[tokio::test]
    async fn acked_end_gets_no_reply_when_the_peer_already_ended() {
        with_timeout(TEST_LIMIT, async {
            // given: the peer finished first, the stream replied with ACK
            let link = Link::new().await;
            link.peer.send(&end_packet(0)).await;
            expect_packet(&link.peer, ACK_OF_PEER_END, "ACK for peer END").await;
            let scenario = async {
                expect_packet(&link.peer, OUR_END_AFTER_PEER_END, "stream END").await;
                // when: the peer acks our END
                link.peer.send(&ack_packet(1, OPEN_WINDOW, &[])).await;
                // then: no more packets
                expect_silence(&link.peer).await;
            };
            let (shutdown, ()) = tokio::join!(link.stream.shutdown(), scenario);
            shutdown.expect("shutdown after ACK");
        })
        .await;
    }
}

// ── setup and helpers ────────────────────────────────────────────────────────

/// The peer's next packet equals `expected_hex` byte for byte (wait up to 1 s).
async fn expect_packet(peer: &RawPeer, expected_hex: &str, what: &str) {
    let (_, packet) = peer
        .recv_within(REPLY_WAIT)
        .await
        .unwrap_or_else(|| panic!("{what}: no packet within {REPLY_WAIT:?}"));
    assert_eq!(hex(&packet), expected_hex, "{what}");
}

/// The peer receives no packets for 500 ms.
async fn expect_silence(peer: &RawPeer) {
    let extra = peer.collect_for(QUIET).await;
    let hexes: Vec<String> = extra.iter().map(|(_, packet)| hex(packet)).collect();
    assert!(extra.is_empty(), "extra packets: {hexes:?}");
}
