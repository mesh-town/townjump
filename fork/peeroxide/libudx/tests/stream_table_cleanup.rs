// JS: libudx/src/udx.c (the stream entry is removed from the socket table when the adapter is abandoned)
#![deny(clippy::all)]

mod common;

use std::time::Duration;

use common::raw_peer::{Link, STREAM_ID, data_packet};
use common::with_timeout;

const WAIT: Duration = Duration::from_secs(1);

#[tokio::test]
async fn packet_to_dropped_stream_id_goes_to_fallback() {
    with_timeout(Duration::from_secs(15), async {
        // given: a connected stream is handed to the adapter, the socket is receiving
        let link = Link::new().await;
        let Link {
            peer,
            stream,
            keep_alive,
        } = link;
        let mut fallback = keep_alive.1.recv_start().expect("recv_start");
        // when: the adapter is abandoned without closing, the peer sends to the stream id (STREAM_ID)
        drop(stream.into_async_stream());
        peer.send(&data_packet(0, b"late")).await;
        // then: the entry is removed, the datagram goes to the fallback channel, not to the stream's dead channel
        let got = tokio::time::timeout(WAIT, fallback.recv()).await;
        let packet = got.unwrap_or_else(|_| panic!("stream {STREAM_ID} entry not removed"));
        assert!(packet.is_some(), "fallback channel closed");
    })
    .await;
}
