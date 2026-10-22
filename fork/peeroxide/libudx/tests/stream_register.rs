// JS: libudx/src/udx.c (the stream is visible to the socket from creation, :2366)
#![deny(clippy::all)]

mod common;

use std::time::Duration;

use common::raw_peer::{Link, PEER_ID, STREAM_ID, ack_of, data_packet};
use common::{create_bound_socket, with_timeout};

const QUIET: Duration = Duration::from_millis(200);
const WAIT: Duration = Duration::from_secs(1);

#[tokio::test]
async fn packet_before_connect_is_read_after() {
    with_timeout(Duration::from_secs(15), async {
        // given: the stream is registered, not connected; the receive loop on the socket is running
        let mut link = Link::registered().await;
        let mut fallback = link.keep_alive.1.recv_start().expect("recv_start");
        // when: the peer sends DATA before connect, and then the stream is connected
        link.peer.send(&data_packet(0, b"early")).await;
        let none = tokio::time::timeout(QUIET, fallback.recv()).await;
        assert!(
            none.is_err(),
            "the packet went to the socket's fallback channel"
        );
        link.connect_to_peer().await;
        // then: the packet is readable, the peer receives an ACK
        let got = tokio::time::timeout(WAIT, link.stream.read()).await;
        let got = got.expect("read timeout").expect("read").expect("eof");
        assert_eq!(got, b"early");
        let (_, ack) = link.peer.recv_within(QUIET).await.expect("no ACK");
        assert_eq!(ack_of(&ack), 1, "the ACK does not cover the early packet");
    })
    .await;
}

#[tokio::test]
async fn dropped_registered_stream_frees_its_id() {
    with_timeout(Duration::from_secs(15), async {
        // given: the stream is registered, the receive loop is running
        let link = Link::registered().await;
        let Link {
            peer,
            stream,
            keep_alive,
        } = link;
        let mut fallback = keep_alive.1.recv_start().expect("recv_start");
        // when: the stream is dropped, the peer sends to its id
        drop(stream);
        peer.send(&data_packet(0, b"late")).await;
        // then: the datagram goes to the fallback channel
        let got = tokio::time::timeout(WAIT, fallback.recv()).await;
        assert!(got.expect("id not released").is_some());
    })
    .await;
}

#[tokio::test]
async fn connect_on_another_socket_fails_and_keeps_the_entry() {
    with_timeout(Duration::from_secs(15), async {
        // given: the stream is registered on A, A is receiving
        let link = Link::registered().await;
        let mut fallback = link.keep_alive.1.recv_start().expect("recv_start");
        let (socket_b, _) = create_bound_socket(&link.keep_alive.0).await;
        // when: connect on B returns Err, the peer sends a packet with id 5 to A
        let res = link
            .stream
            .connect(&socket_b, PEER_ID, link.peer.addr())
            .await;
        assert!(res.is_err(), "connect on a foreign socket succeeded");
        link.peer.send(&data_packet(0, b"x")).await;
        // then: the entry on A is intact, the packet is not in the fallback channel
        let none = tokio::time::timeout(QUIET, fallback.recv()).await;
        assert!(none.is_err(), "the entry on A was lost (id {STREAM_ID})");
    })
    .await;
}

#[tokio::test]
async fn register_on_another_socket_fails_and_keeps_the_entry() {
    with_timeout(Duration::from_secs(15), async {
        // given: the stream is registered on A, A is receiving
        let link = Link::registered().await;
        let mut fallback = link.keep_alive.1.recv_start().expect("recv_start");
        let (socket_b, _) = create_bound_socket(&link.keep_alive.0).await;
        // when: register on B returns Err, the peer sends a packet with id 5 to A
        assert!(
            link.stream.register(&socket_b).is_err(),
            "register on a foreign socket succeeded"
        );
        link.peer.send(&data_packet(0, b"x")).await;
        // then: the entry on A is intact, the packet is not in the fallback channel
        let none = tokio::time::timeout(QUIET, fallback.recv()).await;
        assert!(none.is_err(), "the entry on A was lost (id {STREAM_ID})");
    })
    .await;
}
