// Tests of the `incoming` module: server incoming-connection queue — order, waiting, close, backpressure.
// The sender is the server implementation's side, `Incoming` is the consumer's side (`server.on('connection')`).
use super::*;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use tokio::sync::mpsc::error::TrySendError;
use tokio::time::timeout;

const WAIT: Duration = Duration::from_secs(5);
const SHORT: Duration = Duration::from_millis(50);

mod delivery {
    use super::*;

    #[tokio::test]
    async fn queued_connections_come_out_in_send_order() {
        let (tx, mut incoming) = Incoming::channel(3);
        let _peers = [
            send_one(&tx, 1).await,
            send_one(&tx, 2).await,
            send_one(&tx, 3).await,
        ];

        let keys = [
            accept_key(&mut incoming).await,
            accept_key(&mut incoming).await,
            accept_key(&mut incoming).await,
        ];

        assert_eq!(keys, [Some([1; 32]), Some([2; 32]), Some([3; 32])]);
    }

    #[tokio::test]
    async fn accepted_connection_keeps_its_stream() {
        let (tx, mut incoming) = Incoming::channel(1);
        let mut peer = send_one(&tx, 9).await;

        let mut accepted = accept_now(&mut incoming)
            .await
            .expect("connection in the queue");
        peer.write_all(b"hello").await.unwrap();
        let mut buf = [0u8; 5];
        accepted.stream.read_exact(&mut buf).await.unwrap();

        assert_eq!(&buf, b"hello");
    }

    #[tokio::test]
    async fn accept_waits_while_the_queue_is_empty_and_the_server_is_open() {
        let (tx, mut incoming) = Incoming::channel(1);

        let early = timeout(SHORT, incoming.accept()).await;
        let _peer = send_one(&tx, 4).await;

        assert!(
            early.is_err(),
            "an empty queue with a live sender answers neither `Some` nor `None`"
        );
        assert_eq!(accept_key(&mut incoming).await, Some([4; 32]));
    }
}

mod server_closed {
    use super::*;

    #[tokio::test]
    async fn accept_gives_none_when_the_sender_is_dropped() {
        let (tx, mut incoming) = Incoming::channel(1);

        drop(tx);

        assert_eq!(accept_key(&mut incoming).await, None);
    }

    #[tokio::test]
    async fn queued_connections_are_delivered_before_none() {
        let (tx, mut incoming) = Incoming::channel(2);
        let _peer = send_one(&tx, 5).await;

        drop(tx);

        assert_eq!(accept_key(&mut incoming).await, Some([5; 32]));
        assert_eq!(accept_key(&mut incoming).await, None);
    }

    #[tokio::test]
    async fn sender_fails_once_the_consumer_dropped_incoming() {
        let (tx, incoming) = Incoming::channel(1);
        let (conn, _peer) = connection(6);

        drop(incoming);

        assert!(
            tx.send(conn).await.is_err(),
            "the server must learn that nobody is accepting"
        );
    }
}

mod capacity {
    use super::*;

    #[tokio::test]
    async fn full_queue_refuses_a_new_connection_until_one_is_accepted() {
        let (tx, mut incoming) = Incoming::channel(1);
        let (first, _peer1) = connection(1);
        let (second, _peer2) = connection(2);
        let (third, _peer3) = connection(3);

        tx.try_send(first).unwrap();
        let refused = tx.try_send(second);
        accept_now(&mut incoming)
            .await
            .expect("first connection in the queue");
        let after_accept = tx.try_send(third);

        assert!(
            matches!(refused, Err(TrySendError::Full(_))),
            "queue of 1 is full"
        );
        assert!(after_accept.is_ok(), "room was freed after accept");
    }
}

fn connection(tag: u8) -> (DhtConnection, DuplexStream) {
    let (ours, peer) = tokio::io::duplex(64);
    (DhtConnection::new([tag; 32], Box::new(ours)), peer)
}

async fn send_one(tx: &mpsc::Sender<DhtConnection>, tag: u8) -> DuplexStream {
    let (conn, peer) = connection(tag);
    tx.send(conn).await.unwrap();
    peer
}

async fn accept_now(incoming: &mut Incoming) -> Option<DhtConnection> {
    timeout(WAIT, incoming.accept())
        .await
        .expect("accept did not return in time")
}

async fn accept_key(incoming: &mut Incoming) -> Option<[u8; 32]> {
    accept_now(incoming)
        .await
        .map(|conn| conn.remote_public_key)
}
