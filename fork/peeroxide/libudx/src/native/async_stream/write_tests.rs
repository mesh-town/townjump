// Tests of `InFlight` (accounting of writes without ACK). End-to-end ones are in `tests/async_stream_io.rs`.
// JS: udx-native/lib/stream.js

use std::future::poll_fn;
use std::io;
use std::task::Poll;

use tokio::sync::oneshot;

use super::InFlight;
use crate::error::UdxError;

/// One `poll_settle` poll: the result as is, without waiting.
async fn settle_once(in_flight: &mut InFlight) -> Poll<io::Result<()>> {
    poll_fn(|cx| Poll::Ready(in_flight.poll_settle(cx))).await
}

#[tokio::test]
async fn empty_in_flight_settles_ready() {
    let mut in_flight = InFlight::default();
    assert!(matches!(
        settle_once(&mut in_flight).await,
        Poll::Ready(Ok(()))
    ));
}

#[tokio::test]
async fn settle_pops_acked_head_only() {
    let mut in_flight = InFlight::default();
    let (tx1, rx1) = oneshot::channel();
    let (tx2, rx2) = oneshot::channel();
    in_flight.push(rx1);
    in_flight.push(rx2);
    tx1.send(Ok(())).expect("send ack");
    assert!(matches!(settle_once(&mut in_flight).await, Poll::Pending));
    // head removed, the second receiver is still waiting for its ACK: the next poll is Pending again
    assert!(matches!(settle_once(&mut in_flight).await, Poll::Pending));
    assert_eq!(in_flight.acks.len(), 1);
    drop(tx2);
    assert!(matches!(
        settle_once(&mut in_flight).await,
        Poll::Ready(Err(_))
    ));
}

#[tokio::test]
async fn settle_waits_for_head_in_order() {
    let mut in_flight = InFlight::default();
    let (tx1, rx1) = oneshot::channel();
    let (tx2, rx2) = oneshot::channel();
    in_flight.push(rx1);
    in_flight.push(rx2);
    tx2.send(Ok(())).expect("send ack");
    assert!(matches!(settle_once(&mut in_flight).await, Poll::Pending));
    drop(tx1);
    assert!(matches!(
        settle_once(&mut in_flight).await,
        Poll::Ready(Err(_))
    ));
}

#[tokio::test]
async fn two_writes_acked_in_order_settle_ready() {
    let mut in_flight = InFlight::default();
    let (tx1, rx1) = oneshot::channel();
    let (tx2, rx2) = oneshot::channel();
    in_flight.push(rx1);
    in_flight.push(rx2);
    tx1.send(Ok(())).expect("send ack");
    tx2.send(Ok(())).expect("send ack");
    assert!(matches!(
        settle_once(&mut in_flight).await,
        Poll::Ready(Ok(()))
    ));
    assert_eq!(in_flight.acks.len(), 0);
}

#[tokio::test]
async fn write_error_surfaces_as_other() {
    let mut in_flight = InFlight::default();
    let (tx, rx) = oneshot::channel();
    in_flight.push(rx);
    tx.send(Err(UdxError::StreamClosed)).expect("send error");
    let Poll::Ready(Err(e)) = settle_once(&mut in_flight).await else {
        panic!("expected Ready(Err)");
    };
    assert_eq!(e.kind(), io::ErrorKind::Other);
    assert!(matches!(
        settle_once(&mut in_flight).await,
        Poll::Ready(Ok(()))
    ));
}

#[tokio::test]
async fn dropped_sender_is_broken_pipe() {
    let mut in_flight = InFlight::default();
    let (tx, rx) = oneshot::channel::<crate::error::Result<()>>();
    in_flight.push(rx);
    drop(tx);
    let Poll::Ready(Err(e)) = settle_once(&mut in_flight).await else {
        panic!("expected Ready(Err)");
    };
    assert_eq!(e.kind(), io::ErrorKind::BrokenPipe);
}
