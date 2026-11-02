// JS: libudx/src/udx.c (TIME_WAIT is a TODO there, udx.c:442; the fork keeps a closed stream's entry as a tombstone)
//! A repeated END of the peer after our side is closed: ACK when both ENDs are in order, DESTROY otherwise.
#![deny(clippy::all)]

mod common;

use std::time::Duration;

use common::raw_peer::{
    FLAG_DESTROY, FLAG_END, Link, OPEN_WINDOW, RawPeer, Received, ack_of, ack_packet, end_packet,
    flags_of,
};
use common::with_timeout;
use libudx::UdxAsyncStream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const TEST_LIMIT: Duration = Duration::from_secs(20);
const SETTLE: Duration = Duration::from_millis(150);

fn has_destroy(packets: &[Received]) -> bool {
    packets.iter().any(|(_, p)| flags_of(p) & FLAG_DESTROY != 0)
}

/// `shutdown()` of the adapter while the peer acks our END (ack 1).
async fn shutdown_acked_by(adapter: &mut UdxAsyncStream, peer: &RawPeer) {
    let ack_our_end = async {
        loop {
            let (_, packet) = peer
                .recv_within(Duration::from_secs(2))
                .await
                .expect("no END from the stream");
            if flags_of(&packet) & FLAG_END != 0 {
                peer.send(&ack_packet(1, OPEN_WINDOW, &[])).await;
                break;
            }
        }
    };
    let (shutdown, ()) = tokio::join!(adapter.shutdown(), ack_our_end);
    shutdown.expect("shutdown");
}

mod end_to_end {
    use super::*;

    #[tokio::test]
    async fn repeated_peer_end_is_acked_after_final_ack_was_lost() {
        with_timeout(TEST_LIMIT, async {
            // given: END from both sides is exchanged, the adapter is dropped (path 1: finished)
            let Link {
                peer,
                stream,
                keep_alive: _keep,
            } = Link::new().await;
            let mut adapter = stream.into_async_stream();
            peer.send(&end_packet(0)).await;
            let mut buf = [0u8; 16];
            assert_eq!(adapter.read(&mut buf).await.expect("read"), 0, "no EOF");
            shutdown_acked_by(&mut adapter, &peer).await;
            drop(adapter);
            tokio::time::sleep(SETTLE).await;
            peer.collect_for(SETTLE).await;
            // when: the peer repeats its END
            peer.send(&end_packet(0)).await;
            // then: ACK covering it, no DESTROY
            let got = peer.collect_for(Duration::from_secs(1)).await;
            assert!(!has_destroy(&got), "DESTROY instead of ACK: {got:?}");
            let acks: Vec<_> = got.iter().filter(|(_, p)| ack_of(p) == 1).collect();
            assert!(!acks.is_empty(), "no ACK with ack 1: {got:?}");
        })
        .await;
    }
}

mod mirror {
    use super::*;

    /// Our END is acked by the peer, the peer's END has not come; the adapter is dropped (path 3).
    async fn our_end_acked_adapter_dropped() -> (RawPeer, impl Sized) {
        let Link {
            peer,
            stream,
            keep_alive,
        } = Link::new().await;
        let mut adapter = stream.into_async_stream();
        shutdown_acked_by(&mut adapter, &peer).await;
        drop(adapter);
        tokio::time::sleep(SETTLE).await;
        // the peer throws away everything it has received (END, final ACK, DESTROY)
        peer.collect_for(SETTLE).await;
        (peer, keep_alive)
    }

    #[tokio::test]
    async fn peer_end_in_order_is_acked() {
        with_timeout(TEST_LIMIT, async {
            let (peer, _keep) = our_end_acked_adapter_dropped().await;
            // when: the peer's END arrives in order
            peer.send(&end_packet(0)).await;
            // then: ACK with ack 1, no DESTROY
            let got = peer.collect_for(Duration::from_secs(1)).await;
            assert!(!has_destroy(&got), "DESTROY instead of ACK: {got:?}");
            assert!(got.iter().any(|(_, p)| ack_of(p) == 1), "no ACK: {got:?}");
        })
        .await;
    }

    #[tokio::test]
    async fn peer_end_with_a_gap_gets_destroy() {
        with_timeout(TEST_LIMIT, async {
            let (peer, _keep) = our_end_acked_adapter_dropped().await;
            // when: the peer's END arrives with a gap (seq 3)
            peer.send(&end_packet(3)).await;
            // then: DESTROY
            let got = peer.collect_for(Duration::from_secs(1)).await;
            assert!(has_destroy(&got), "no DESTROY: {got:?}");
        })
        .await;
    }
}
