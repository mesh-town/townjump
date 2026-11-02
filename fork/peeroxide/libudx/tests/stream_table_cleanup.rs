// JS: libudx/src/udx.c (the closed stream's entry lingers as a tombstone, then goes back to the fallback)
#![deny(clippy::all)]

mod common;

use std::time::Duration;

use common::raw_peer::{FLAG_DESTROY, Link, STREAM_ID, data_packet, flags_of};
use common::with_timeout;

const WAIT: Duration = Duration::from_secs(1);

#[tokio::test]
async fn packet_to_dropped_stream_id_goes_to_fallback_after_linger() {
    with_timeout(Duration::from_secs(120), async {
        // given: a connected stream is handed to the adapter, the socket is receiving
        let link = Link::new().await;
        let Link {
            peer,
            stream,
            keep_alive,
        } = link;
        let mut fallback = keep_alive.1.recv_start().expect("recv_start");
        // when: the adapter is abandoned without closing, the peer sends to the stream id (STREAM_ID) at once
        drop(stream.into_async_stream());
        let (_, first) = peer.recv_within(WAIT).await.expect("no DESTROY from drop");
        assert_eq!(flags_of(&first), FLAG_DESTROY);
        tokio::time::sleep(Duration::from_millis(50)).await;
        peer.send(&data_packet(0, b"late")).await;
        // then: the tombstone holds the id: the fallback stays empty, the peer gets DESTROY
        let got = tokio::time::timeout(WAIT, fallback.recv()).await;
        assert!(
            got.is_err(),
            "stream {STREAM_ID} packet reached the fallback inside the linger"
        );
        let (_, reply) = peer
            .recv_within(WAIT)
            .await
            .expect("no DESTROY from the tombstone");
        assert_eq!(flags_of(&reply), FLAG_DESTROY);
        // when: the tombstone has expired (rto 1 s on the loopback: 20 s; the clock is run past it, 1 s steps), the peer repeats
        tokio::time::pause();
        for _ in 0..25 {
            tokio::time::advance(Duration::from_secs(1)).await;
        }
        tokio::time::resume();
        peer.send(&data_packet(0, b"late")).await;
        // then: the packet goes to the fallback, the peer hears nothing
        let got = tokio::time::timeout(WAIT, fallback.recv()).await;
        let packet = got.unwrap_or_else(|_| panic!("stream {STREAM_ID} entry not removed"));
        assert!(packet.is_some(), "fallback channel closed");
        assert!(peer.recv_within(Duration::from_millis(300)).await.is_none());
    })
    .await;
}
