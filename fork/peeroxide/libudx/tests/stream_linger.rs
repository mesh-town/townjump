// JS: libudx/src/udx.c (TIME_WAIT is a TODO there, udx.c:442; the fork keeps a closed stream's entry as a tombstone)
//! Drop of the adapter with END in flight: the peer must learn about the close even if the first DESTROY is lost.
#![deny(clippy::all)]

mod common;

use std::time::Duration;

use common::raw_peer::{FLAG_DESTROY, FLAG_END, Link, Received, data_packet, flags_of};
use common::with_timeout;
use libudx::{UdxAsyncStream, UdxStream};
use tokio::io::AsyncWriteExt;

const TEST_LIMIT: Duration = Duration::from_secs(20);

fn count_flag(packets: &[Received], flag: u8) -> usize {
    packets
        .iter()
        .filter(|(_, packet)| flags_of(packet) & flag != 0)
        .count()
}

fn has_destroy(packets: &[Received]) -> bool {
    count_flag(packets, FLAG_DESTROY) > 0
}

/// An adapter with one DATA and our END in flight; the peer never acks, so the shutdown timeout is expected.
async fn adapter_with_end_in_flight(stream: UdxStream) -> UdxAsyncStream {
    let mut adapter = stream.into_async_stream();
    adapter.write_all(b"x").await.expect("write");
    let shutdown = tokio::time::timeout(Duration::from_millis(300), adapter.shutdown()).await;
    assert!(
        shutdown.is_err(),
        "shutdown finished without the peer's ACK"
    );
    adapter
}

mod drop_with_end_in_flight {
    use super::*;

    #[tokio::test]
    async fn reaches_peer_with_destroy() {
        with_timeout(TEST_LIMIT, async {
            // given: an adapter with DATA and END in flight, the peer has seen them
            let Link {
                peer,
                stream,
                keep_alive: _keep,
            } = Link::new().await;
            let adapter = adapter_with_end_in_flight(stream).await;
            peer.collect_for(Duration::from_millis(50)).await;
            // when: the adapter is dropped
            drop(adapter);
            // then: within 1.1 s the peer gets DESTROY or at least two END repeats
            let got = peer.collect_for(Duration::from_millis(1100)).await;
            assert!(
                has_destroy(&got) || count_flag(&got, FLAG_END) >= 2,
                "no DESTROY and no END repeats: {got:?}"
            );
        })
        .await;
    }

    #[tokio::test]
    async fn answers_peer_retry_after_lost_destroy() {
        with_timeout(TEST_LIMIT, async {
            // given: the same adapter, the first DESTROY reached the peer and was thrown away
            let Link {
                peer,
                stream,
                keep_alive: _keep,
            } = Link::new().await;
            let adapter = adapter_with_end_in_flight(stream).await;
            peer.collect_for(Duration::from_millis(50)).await;
            drop(adapter);
            let first = peer.collect_for(Duration::from_millis(1100)).await;
            assert!(
                has_destroy(&first),
                "the first DESTROY did not arrive: {first:?}"
            );
            tokio::time::sleep(Duration::from_millis(200)).await;
            // when: the peer repeats DATA
            peer.send(&data_packet(0, b"retry")).await;
            // then: within 1 s DESTROY arrives again
            let again = peer.collect_for(Duration::from_secs(1)).await;
            assert!(has_destroy(&again), "no DESTROY for the retry: {again:?}");
        })
        .await;
    }
}

mod late_data_to_abandoned_stream {
    use super::*;
    use common::raw_peer::{ack_of, seq_of};

    #[tokio::test]
    async fn gets_destroy_at_most_once_per_rto() {
        with_timeout(TEST_LIMIT, async {
            // given: the adapter is dropped, the peer threw the first DESTROY away
            let Link {
                peer,
                stream,
                keep_alive: _keep,
            } = Link::new().await;
            drop(stream.into_async_stream());
            let first = peer.recv_within(Duration::from_secs(1)).await;
            assert!(
                first.is_some_and(|(_, p)| flags_of(&p) == FLAG_DESTROY),
                "no first DESTROY"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
            // when: a late DATA arrives
            peer.send(&data_packet(0, b"late")).await;
            // then: DESTROY with our final numbers
            let (_, reply) = peer
                .recv_within(Duration::from_secs(1))
                .await
                .expect("no DESTROY for the late DATA");
            assert_eq!(flags_of(&reply), FLAG_DESTROY);
            assert_eq!((seq_of(&reply), ack_of(&reply)), (0, 0));
            // when: a second DATA within the rto; then: silence
            peer.send(&data_packet(0, b"late")).await;
            let quiet = peer.collect_for(Duration::from_millis(500)).await;
            assert!(quiet.is_empty(), "second reply inside the rto: {quiet:?}");
            // when: a third DATA 1.1 s after the first reply; then: DESTROY again
            tokio::time::sleep(Duration::from_millis(600)).await;
            peer.send(&data_packet(0, b"late")).await;
            let again = peer.collect_for(Duration::from_secs(1)).await;
            assert!(has_destroy(&again), "no DESTROY after the rto: {again:?}");
        })
        .await;
    }

    #[tokio::test]
    async fn from_another_address_gets_silence() {
        with_timeout(TEST_LIMIT, async {
            // given: the adapter is dropped, the first DESTROY reached the peer
            let Link {
                peer,
                stream,
                keep_alive,
            } = Link::new().await;
            drop(stream.into_async_stream());
            assert!(peer.recv_within(Duration::from_secs(1)).await.is_some());
            tokio::time::sleep(Duration::from_millis(50)).await;
            // when: a stranger sends DATA with the same id
            let stranger = tokio::net::UdpSocket::bind("127.0.0.1:0")
                .await
                .expect("bind stranger");
            let target = keep_alive.1.local_addr().await.expect("socket addr");
            stranger
                .send_to(&data_packet(0, b"late"), target)
                .await
                .expect("send from stranger");
            // then: the stranger and the real peer hear nothing
            let mut buf = [0u8; 64];
            let heard =
                tokio::time::timeout(Duration::from_secs(1), stranger.recv_from(&mut buf)).await;
            assert!(heard.is_err(), "the stranger got a reply");
            let quiet = peer.collect_for(Duration::from_millis(200)).await;
            assert!(quiet.is_empty(), "the peer got a reply: {quiet:?}");
        })
        .await;
    }
}
