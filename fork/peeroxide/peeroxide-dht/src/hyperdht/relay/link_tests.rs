//! Tests for [`super`]: the link keeps the relay session until the stream closes.

use std::time::Duration;

use libudx::UdxRuntime;

use super::*;

/// A connected stream on a loopback socket (nobody answers on the far side).
async fn stream_pair() -> (UdxRuntime, libudx::UdxStream) {
    let runtime = UdxRuntime::new().expect("runtime");
    let socket = runtime.create_socket().await.expect("socket");
    socket
        .bind("127.0.0.1:0".parse().unwrap())
        .await
        .expect("bind");
    let stream = runtime.create_stream(1).await.expect("stream");
    stream
        .connect(&socket, 100, "127.0.0.1:9".parse().unwrap())
        .await
        .expect("connect");
    (runtime, stream)
}

#[tokio::test]
async fn relayed_only_keeps_session_until_stream_closed() {
    let (_rt, stream) = stream_pair().await;
    let (session, mut closed) = RelaySession::probe();
    let link = RelayLink::relayed_only(stream.control(), session);

    let early = tokio::time::timeout(Duration::from_millis(300), &mut closed).await;
    assert!(early.is_err(), "session closed while the stream is open");

    stream.destroy().await.expect("destroy");
    let late = tokio::time::timeout(Duration::from_secs(2), closed).await;
    assert!(
        matches!(late, Ok(Ok(()))),
        "session must close with the stream"
    );
    drop(link);
}

#[tokio::test]
async fn relayed_only_path_is_relayed() {
    let (_rt, stream) = stream_pair().await;
    let (session, _closed) = RelaySession::probe();
    let link = RelayLink::relayed_only(stream.control(), session);
    assert_eq!(*link.path().borrow(), RelayPath::Relayed);
}
