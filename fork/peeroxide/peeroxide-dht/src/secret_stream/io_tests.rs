// Tests for the AsyncRead/AsyncWrite path of SecretStream (io.rs).
// Trait calls are fully qualified where an inherent read()/write()/shutdown() would shadow them.

use std::future::{Future, poll_fn};
use std::io::ErrorKind;
use std::pin::Pin;
use std::sync::atomic::Ordering;
use std::task::Poll;
use std::time::Duration;

use rand::Rng;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use super::super::test_support::session_pair;

async fn within<F: Future>(f: F) -> F::Output {
    tokio::time::timeout(Duration::from_secs(20), f)
        .await
        .expect("test timed out")
}

async fn read_chunk<R: AsyncRead + Unpin>(r: &mut R, size: usize) -> std::io::Result<Vec<u8>> {
    let mut buf = vec![0u8; size];
    let n = AsyncReadExt::read(r, &mut buf).await?;
    buf.truncate(n);
    Ok(buf)
}

async fn read_until_eof<R: AsyncRead + Unpin>(r: &mut R, size: usize) -> Vec<u8> {
    let mut all = Vec::new();
    loop {
        let chunk = read_chunk(r, size).await.unwrap();
        if chunk.is_empty() {
            return all;
        }
        all.extend_from_slice(&chunk);
    }
}

#[tokio::test]
async fn megabyte_roundtrip_read_in_odd_chunks() {
    let ((mut a, _pa), (mut b, _pb)) = session_pair(8192).await;
    let mut data = vec![0u8; 1 << 20];
    rand::rng().fill(&mut data[..]);

    let writer = async {
        tokio::io::copy(&mut &data[..], &mut a).await.unwrap();
        AsyncWriteExt::shutdown(&mut a).await.unwrap();
    };
    let (_, received) = within(async { tokio::join!(writer, read_until_eof(&mut b, 7777)) }).await;
    assert_eq!(received, data);
}

#[tokio::test]
async fn small_buffer_drains_message_over_reads() {
    let ((mut a, _pa), (mut b, _pb)) = session_pair(8192).await;
    let data: Vec<u8> = (0..100u8).collect();
    a.write_all(&data).await.unwrap();

    let mut collected = Vec::new();
    for _ in 0..10 {
        let chunk = within(read_chunk(&mut b, 10)).await.unwrap();
        assert_eq!(chunk.len(), 10);
        collected.extend_from_slice(&chunk);
    }
    assert_eq!(collected, data);
}

#[tokio::test]
async fn one_poll_write_per_frame() {
    let ((mut a, pa), (_b, _pb)) = session_pair(8192).await;
    pa.writes.store(0, Ordering::SeqCst);

    a.write_all(&[1u8; 100]).await.unwrap();
    assert_eq!(pa.writes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn empty_write_sends_nothing() {
    let ((mut a, pa), (_b, _pb)) = session_pair(8192).await;
    pa.writes.store(0, Ordering::SeqCst);

    let n = poll_fn(|cx| Pin::new(&mut a).poll_write(cx, &[]))
        .await
        .unwrap();
    assert_eq!(n, 0);
    assert_eq!(pa.writes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn keepalive_frame_is_skipped() {
    let ((mut a, _pa), (mut b, _pb)) = session_pair(8192).await;
    a.write(&[]).await.unwrap(); // inherent: one keepalive frame
    a.write_all(b"abc").await.unwrap();

    let chunk = within(read_chunk(&mut b, 16)).await.unwrap();
    assert_eq!(chunk, b"abc");
}

#[tokio::test]
async fn eof_on_frame_boundary_is_clean() {
    let ((mut a, _pa), (mut b, _pb)) = session_pair(8192).await;
    AsyncWriteExt::shutdown(&mut a).await.unwrap();

    assert!(within(read_chunk(&mut b, 16)).await.unwrap().is_empty());
}

#[tokio::test]
async fn eof_inside_frame_is_unexpected_eof() {
    let ((a, _pa), (mut b, _pb)) = session_pair(8192).await;
    let mut raw = a.into_inner();
    raw.write_all(&[100, 0, 0]).await.unwrap();
    raw.write_all(&[0u8; 10]).await.unwrap();
    raw.shutdown().await.unwrap();

    let err = within(read_chunk(&mut b, 16)).await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::UnexpectedEof);
}

#[tokio::test]
async fn raw_error_passes_through() {
    let ((_a, _pa), (mut b, pb)) = session_pair(8192).await;
    pb.fail_reads.store(true, Ordering::SeqCst);

    let err = within(read_chunk(&mut b, 16)).await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::ConnectionReset);
}

/// Send `body` behind a valid 3-byte header straight into the transport of B.
async fn inject_frame_and_read(body: &[u8]) -> std::io::Error {
    let ((a, _pa), (mut b, _pb)) = session_pair(8192).await;
    let mut raw = a.into_inner();
    raw.write_all(&[body.len() as u8, 0, 0]).await.unwrap();
    raw.write_all(body).await.unwrap();

    within(read_chunk(&mut b, 16)).await.unwrap_err()
}

#[tokio::test]
async fn short_frame_is_invalid_data() {
    let err = inject_frame_and_read(&[1, 2, 3, 4, 5]).await;
    assert_eq!(err.kind(), ErrorKind::InvalidData);
}

#[tokio::test]
async fn corrupt_frame_is_invalid_data() {
    let err = inject_frame_and_read(&[0xAA; 40]).await;
    assert_eq!(err.kind(), ErrorKind::InvalidData);
}

/// A writes 10 000 bytes through `poll_write` into a 64-byte pipe (the transport
/// takes a part), then closes; B must still receive everything, then EOF.
async fn shutdown_delivers_pending_frame(inherent: bool) {
    let ((mut a, _pa), (mut b, _pb)) = session_pair(64).await;
    let data = vec![0x5Au8; 10_000];
    let n = poll_fn(|cx| Pin::new(&mut a).poll_write(cx, &data))
        .await
        .unwrap();
    assert_eq!(n, data.len());

    let close = async {
        if inherent {
            a.shutdown().await.unwrap();
        } else {
            AsyncWriteExt::shutdown(&mut a).await.unwrap();
        }
    };
    let mut received = Vec::new();
    let read = b.read_to_end(&mut received);
    let (_, read_result) = within(async { tokio::join!(close, read) }).await;
    read_result.unwrap();
    assert_eq!(received, data);
}

#[tokio::test]
async fn shutdown_flushes_pending_frame() {
    shutdown_delivers_pending_frame(false).await;
    shutdown_delivers_pending_frame(true).await;
}

/// The first `poll_write` takes the whole buffer even if the pipe is full;
/// the next one waits until the unsent rest of that frame is drained.
#[tokio::test]
async fn write_is_pending_while_previous_frame_unsent() {
    let ((mut a, _pa), (mut b, _pb)) = session_pair(64).await;
    let data = vec![1u8; 5_000];
    let first = poll_fn(|cx| Poll::Ready(Pin::new(&mut a).poll_write(cx, &data))).await;
    assert!(matches!(first, Poll::Ready(Ok(5_000))));
    let second = poll_fn(|cx| Poll::Ready(Pin::new(&mut a).poll_write(cx, b"z"))).await;
    assert!(second.is_pending());

    // Flushing A drains the rest of the frame while B reads it.
    let mut drained = vec![0u8; 5_000];
    let (flushed, read) =
        within(async { tokio::join!(a.flush(), b.read_exact(&mut drained)) }).await;
    flushed.unwrap();
    read.unwrap();
    assert_eq!(drained, data);
}
