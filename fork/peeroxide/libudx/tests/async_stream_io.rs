#![deny(clippy::all)]

//! `UdxAsyncStream` write without waiting for ACK and close (FIN, drop, `abort`).
//! Tests run on `current_thread` (on `multi_thread` ACKs arrive inside one `poll_fn`); the exception is
//! `drop_while_peer_blocked_in_read_resets`, which needs exactly `multi_thread`.
// JS: udx-native/lib/stream.js

mod common;

use std::future::poll_fn;
use std::io;
use std::pin::Pin;
use std::task::Poll;
use std::time::Duration;

use libudx::{UdxAsyncStream, UdxRuntime, UdxSocket};
use tokio::io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// A pair of streams A(1) ↔ B(2); the runtime and sockets are held while the pair exists.
struct Pair {
    a: UdxAsyncStream,
    b: UdxAsyncStream,
    _keep: (UdxRuntime, UdxSocket, UdxSocket),
}

async fn pair() -> Pair {
    let runtime = common::create_runtime();
    let (stream_a, stream_b, socket_a, socket_b) = common::create_connected_pair(&runtime).await;
    Pair {
        a: stream_a.into_async_stream(),
        b: stream_b.into_async_stream(),
        _keep: (runtime, socket_a, socket_b),
    }
}

/// Reads once into a 16-byte buffer and returns the result as is.
async fn read_once(stream: &mut UdxAsyncStream) -> io::Result<usize> {
    let mut buf = [0u8; 16];
    stream.read(&mut buf).await
}

#[tokio::test]
async fn sixteen_writes_accepted_in_one_poll_without_ack() {
    let _ = tracing_subscriber::fmt::try_init();
    common::with_timeout(common::DEFAULT_TIMEOUT, async {
        let Pair {
            mut a,
            mut b,
            _keep,
        } = pair().await;
        let payload = common::random_payload(16 * 1024);

        poll_fn(|cx| {
            for chunk in payload.chunks(1024) {
                let polled = Pin::new(&mut a).poll_write(cx, chunk);
                assert!(matches!(polled, Poll::Ready(Ok(1024))), "write: {polled:?}");
            }
            Poll::Ready(())
        })
        .await;

        let mut got = vec![0u8; payload.len()];
        b.read_exact(&mut got).await.expect("read_exact");
        common::verify_payload(&payload, &got);
    })
    .await;
}

/// Writes `chunk` in one poll until `Pending` (no more than 100 times); returns the accepted bytes.
/// Every `Ready` must accept exactly `chunk.len()` (receive floor 64 KiB).
async fn fill_until_pending(a: &mut UdxAsyncStream, chunk: &[u8]) -> usize {
    poll_fn(|cx| {
        let mut total = 0;
        for _ in 0..100 {
            match Pin::new(&mut *a).poll_write(cx, chunk) {
                Poll::Ready(Ok(n)) => {
                    assert_eq!(n, chunk.len(), "partial accept with headroom > 0");
                    total += n;
                }
                Poll::Pending => return Poll::Ready(total),
                other => panic!("write: {other:?}"),
            }
        }
        panic!("write did not hit the budget within 100 chunks");
    })
    .await
}

#[tokio::test]
async fn writes_pause_at_write_budget() {
    let _ = tracing_subscriber::fmt::try_init();
    // The boundary before ACK takes seconds in a debug build: headroom over the overall 15 s timeout.
    common::with_timeout(Duration::from_secs(60), async {
        let Pair {
            mut a,
            b: _b,
            _keep,
        } = pair().await;
        let chunk = common::random_payload(64 * 1024);

        let t = fill_until_pending(&mut a, &chunk).await;

        let low = 262_144;
        assert!(
            (low..=low + 10 * 1500 + 65_536).contains(&t),
            "accepted before the pause: {t}"
        );
        a.flush().await.expect("flush");
    })
    .await;
}

#[tokio::test]
async fn single_write_is_capped_at_write_budget() {
    let _ = tracing_subscriber::fmt::try_init();
    common::with_timeout(common::DEFAULT_TIMEOUT, async {
        let Pair {
            mut a,
            b: _b,
            _keep,
        } = pair().await;
        let big = vec![7u8; 4 * 1024 * 1024 + 10];

        let polled = poll_fn(|cx| Poll::Ready(Pin::new(&mut a).poll_write(cx, &big))).await;

        assert_eq!(UdxAsyncStream::WRITE_HIGH_WATER, 262_144);
        let Poll::Ready(Ok(n)) = polled else {
            panic!("write: {polled:?}");
        };
        assert!(262_144 < n && n <= 262_144 + 10 * 1500, "accepted: {n}");
    })
    .await;
}

#[tokio::test]
async fn parked_writer_resumes_on_acks_without_flush() {
    let _ = tracing_subscriber::fmt::try_init();
    common::with_timeout(Duration::from_secs(60), async {
        let Pair {
            mut a,
            b: _b,
            _keep,
        } = pair().await;
        let chunk = common::random_payload(64 * 1024);
        fill_until_pending(&mut a, &chunk).await;

        let big = common::random_payload(1024 * 1024);
        tokio::time::timeout(Duration::from_secs(30), a.write_all(&big))
            .await
            .expect("write_all timed out")
            .expect("write_all");
        a.flush().await.expect("flush");
    })
    .await;
}

#[tokio::test]
async fn drop_before_end_resets_peer_reader() {
    let _ = tracing_subscriber::fmt::try_init();
    common::with_timeout(common::DEFAULT_TIMEOUT, async {
        let Pair {
            mut a,
            mut b,
            _keep,
        } = pair().await;
        a.write_all(b"x").await.expect("write");
        a.flush().await.expect("flush");

        drop(a);

        let mut one = [0u8; 1];
        b.read_exact(&mut one).await.expect("read x");
        assert_eq!(&one, b"x");
        for _ in 0..2 {
            let err = read_once(&mut b).await.expect_err("expected a reset");
            assert_eq!(err.kind(), io::ErrorKind::ConnectionReset);
        }
    })
    .await;
}

#[tokio::test]
async fn drop_after_unflushed_write_delivers_then_resets() {
    let _ = tracing_subscriber::fmt::try_init();
    common::with_timeout(common::DEFAULT_TIMEOUT, async {
        let Pair {
            mut a,
            mut b,
            _keep,
        } = pair().await;
        let payload = common::random_payload(100);
        a.write_all(&payload).await.expect("write");

        drop(a);

        let mut got = vec![0u8; payload.len()];
        b.read_exact(&mut got).await.expect("data before DESTROY");
        common::verify_payload(&payload, &got);
        let err = read_once(&mut b).await.expect_err("expected a reset");
        assert_eq!(err.kind(), io::ErrorKind::ConnectionReset);
    })
    .await;
}

#[tokio::test]
async fn abort_after_shutdown_resets_peer() {
    let _ = tracing_subscriber::fmt::try_init();
    common::with_timeout(common::DEFAULT_TIMEOUT, async {
        let Pair {
            mut a,
            mut b,
            _keep,
        } = pair().await;
        a.shutdown().await.expect("shutdown");
        assert_eq!(read_once(&mut b).await.expect("eof"), 0);

        a.abort();

        let result = tokio::time::timeout(Duration::from_secs(5), async {
            b.write_all(b"y").await?;
            b.flush().await
        })
        .await
        .expect("without DESTROY flush would hang until RTO");
        assert!(result.is_err(), "a write to a reset stream must fail");
    })
    .await;
}

#[tokio::test]
async fn peer_fin_is_eof_not_reset() {
    let _ = tracing_subscriber::fmt::try_init();
    common::with_timeout(common::DEFAULT_TIMEOUT, async {
        let Pair {
            mut a,
            mut b,
            _keep,
        } = pair().await;
        a.shutdown().await.expect("shutdown");

        assert_eq!(read_once(&mut b).await.expect("first EOF"), 0);
        assert_eq!(read_once(&mut b).await.expect("second EOF"), 0);
    })
    .await;
}

#[tokio::test]
async fn write_after_peer_reset_fails() {
    let _ = tracing_subscriber::fmt::try_init();
    common::with_timeout(common::DEFAULT_TIMEOUT, async {
        let Pair { a, mut b, _keep } = pair().await;
        drop(a);
        let err = read_once(&mut b).await.expect_err("expected a reset");
        assert_eq!(err.kind(), io::ErrorKind::ConnectionReset);

        let result = b.write_all(b"z").await;

        assert!(result.is_err(), "a write to a reset stream must fail");
    })
    .await;
}

#[tokio::test]
async fn shutdown_then_drop_is_eof_for_lagging_reader() {
    let _ = tracing_subscriber::fmt::try_init();
    common::with_timeout(common::DEFAULT_TIMEOUT, async {
        let Pair {
            mut a,
            mut b,
            _keep,
        } = pair().await;
        let payload = common::random_payload(256 * 1024);
        let to_send = payload.clone();
        let writer = tokio::spawn(async move {
            a.write_all(&to_send).await.expect("write_all");
            a.shutdown().await.expect("shutdown");
        });
        writer.await.expect("writer");
        tokio::time::sleep(Duration::from_millis(100)).await;

        let mut got = vec![0u8; payload.len()];
        b.read_exact(&mut got).await.expect("read_exact");
        common::verify_payload(&payload, &got);
        assert_eq!(read_once(&mut b).await.expect("first EOF"), 0);
        assert_eq!(read_once(&mut b).await.expect("second EOF"), 0);
    })
    .await;
}

/// Race on `multi_thread`: the reader is already waiting in `read` when DESTROY arrives. `End` must not
/// overtake `connected = false`, otherwise `classify_end` takes DESTROY for a FIN and returns EOF.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn drop_while_peer_blocked_in_read_resets() {
    let _ = tracing_subscriber::fmt::try_init();
    common::with_timeout(Duration::from_secs(60), async {
        for round in 0..50 {
            let Pair { a, mut b, _keep } = pair().await;
            let reader = tokio::spawn(async move { read_once(&mut b).await });
            tokio::time::sleep(Duration::from_millis(5)).await;

            drop(a);

            let err = reader
                .await
                .expect("reader")
                .expect_err(&format!("round {round}: expected a reset, not EOF"));
            assert_eq!(err.kind(), io::ErrorKind::ConnectionReset, "round {round}");
        }
    })
    .await;
}

/// One `poll_write` poll per byte: the result as is.
async fn write_byte_once(a: &mut UdxAsyncStream) -> Poll<io::Result<usize>> {
    poll_fn(|cx| Poll::Ready(Pin::new(&mut *a).poll_write(cx, b"x"))).await
}

#[tokio::test]
async fn write_at_zero_room_after_shutdown_is_err_not_pending() {
    let _ = tracing_subscriber::fmt::try_init();
    // The boundary before ACK takes seconds in a debug build: headroom over the overall 15 s timeout.
    common::with_timeout(Duration::from_secs(60), async {
        let Pair {
            mut a,
            b: _b,
            _keep,
        } = pair().await;
        let chunk = common::random_payload(64 * 1024);
        fill_until_pending(&mut a, &chunk).await;
        // headroom 0: without a rejection before parking there would be Pending here
        let paused = write_byte_once(&mut a).await;
        assert!(matches!(paused, Poll::Pending), "headroom 0: {paused:?}");

        // FIN is queued behind the data; shutdown itself waits for ACK, so we poll once
        let _ = poll_fn(|cx| Poll::Ready(Pin::new(&mut a).poll_shutdown(cx))).await;

        let polled = write_byte_once(&mut a).await;
        assert!(matches!(polled, Poll::Ready(Err(_))), "write: {polled:?}");
    })
    .await;
}

#[tokio::test]
async fn write_at_zero_room_after_terminate_is_err_not_pending() {
    let _ = tracing_subscriber::fmt::try_init();
    common::with_timeout(Duration::from_secs(60), async {
        let Pair { mut a, b, _keep } = pair().await;
        let chunk = common::random_payload(64 * 1024);
        fill_until_pending(&mut a, &chunk).await;
        let paused = write_byte_once(&mut a).await;
        assert!(matches!(paused, Poll::Pending), "headroom 0: {paused:?}");

        // DESTROY from the peer terminates the stream on `a` (terminate); read reports a reset
        drop(b);
        let err = read_once(&mut a).await.expect_err("expected a reset");
        assert_eq!(err.kind(), io::ErrorKind::ConnectionReset);

        let polled = write_byte_once(&mut a).await;
        assert!(matches!(polled, Poll::Ready(Err(_))), "write: {polled:?}");
    })
    .await;
}
