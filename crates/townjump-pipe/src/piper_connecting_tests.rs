// Tests of `pipe_connecting` with a `dst` that is not ready (pipe.md, «TCP: connPiper»): reading `connection` before `dst`,
// the buffer before it, close and error in that window. The rest is in piper_tests.rs.
use super::PRE_CONNECT_MAX;
use crate::support::*;
use std::future::{Future, ready};
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, DuplexStream, ReadBuf, duplex};
use tokio::sync::oneshot;

/// Raises the flag when `dst` is dropped (or finished).
struct DropFlag(Flag);

impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.raise();
    }
}

/// `dst` that becomes ready after the gate signal; `dropped` is raised when it is destroyed.
fn gated_dst(
    dropped: Flag,
    local: DuplexStream,
) -> (
    oneshot::Sender<()>,
    impl Future<Output = io::Result<DuplexStream>>,
) {
    let (open, wait) = oneshot::channel::<()>();
    let dst = async move {
        let _guard = DropFlag(dropped);
        let _ = wait.await;
        Ok(local)
    };
    (open, dst)
}

/// `pipe_connecting` over a `duplex` `remote` of capacity `cap`; returns the far end and the join.
fn connecting_piper<F>(
    watch: &Watch,
    dst: F,
    cap: usize,
) -> (DuplexStream, impl Future<Output = ()> + use<F>)
where
    F: Future<Output = io::Result<DuplexStream>>,
{
    let (remote_far, remote_near) = duplex(cap);
    let piper = crate::piper::pipe_connecting(remote_near, dst, watch.opts(), watch.stats.clone());
    (remote_far, until_end(watch.ended.clone(), piper))
}

/// `connection` whose read fails at once.
struct BrokenRead;

impl AsyncRead for BrokenRead {
    fn poll_read(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        _: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Poll::Ready(Err(io::Error::other("read boom")))
    }
}

impl AsyncWrite for BrokenRead {
    fn poll_write(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Poll::Ready(Ok(buf.len()))
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

#[tokio::test]
async fn connection_close_before_dst_drops_dst_and_destroys_without_error() {
    let watch = Watch::new(false);
    let dropped = Flag::default();
    let (_local_far, local_near) = duplex(PIPE_BUF);
    let (_open, dst) = gated_dst(dropped.clone(), local_near);
    let (mut remote_far, piper) = connecting_piper(&watch, dst, PIPE_BUF);
    drive(piper, async {
        remote_far.shutdown().await.unwrap();
        dropped.wait("dst dropped").await;
        watch.ended.wait("join returned").await;
        assert_eq!(watch.destroyed.calls(), vec![None]);
        assert_eq!(watch.loc_rem_cnt(), (0, 0));
    })
    .await;
}

#[tokio::test]
async fn bytes_received_before_dst_reach_it_first() {
    let watch = Watch::new(false);
    let (mut local_far, local_near) = duplex(PIPE_BUF);
    let (open, dst) = gated_dst(Flag::default(), local_near);
    // `duplex(256)`: without reading before `dst`, 1 KiB does not fit into the channel.
    let (mut remote_far, piper) = connecting_piper(&watch, dst, 256);
    let (head, tail) = (pattern(1024, 1), pattern(1024, 2));
    drive(piper, async {
        within("write before dst", remote_far.write_all(&head))
            .await
            .unwrap();
        open.send(()).unwrap();
        remote_far.write_all(&tail).await.unwrap();
        let expected = [head.clone(), tail.clone()].concat();
        assert!(
            read_n(&mut local_far, 2048).await == expected,
            "order violated"
        );
    })
    .await;
}

#[tokio::test]
async fn bytes_beyond_the_early_buffer_wait_and_arrive_in_order() {
    const CAP: usize = 4096;
    let watch = Watch::new(false);
    let (mut local_far, local_near) = duplex(PIPE_BUF);
    let (open, dst) = gated_dst(Flag::default(), local_near);
    let (remote_far, piper) = connecting_piper(&watch, dst, CAP);
    let data = pattern(PRE_CONNECT_MAX + 3 * CAP, 3);
    drive(piper, async {
        // `early` takes PRE_CONNECT_MAX, `duplex` takes CAP more; the rest stops on backpressure.
        let writer = tokio::spawn({
            let data = data.clone();
            let mut remote_far = remote_far;
            async move {
                remote_far.write_all(&data).await.unwrap();
                remote_far
            }
        });
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(
            !writer.is_finished(),
            "buffer ceiling does not hold: the writer reached the end"
        );
        open.send(()).unwrap();
        let got = within("read after dst", read_n(&mut local_far, data.len())).await;
        assert!(got == data, "order or content violated");
        let _remote_far = within("writer after dst", writer).await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn connection_read_error_before_dst_destroys_with_it_and_drops_dst() {
    let watch = Watch::new(false);
    let dropped = Flag::default();
    let (_local_far, local_near) = duplex(PIPE_BUF);
    let (_open, dst) = gated_dst(dropped.clone(), local_near);
    let piper = crate::piper::pipe_connecting(BrokenRead, dst, watch.opts(), watch.stats.clone());
    drive(until_end(watch.ended.clone(), piper), async {
        dropped.wait("dst dropped").await;
        watch.ended.wait("join returned").await;
        let expected = Some((io::ErrorKind::Other, "read boom".to_string()));
        assert_eq!(watch.destroyed.calls(), vec![expected]);
    })
    .await;
}

#[tokio::test]
async fn connection_close_with_early_bytes_logs_how_many_were_dropped() {
    let watch = Watch::new(true);
    let (_local_far, local_near) = duplex(PIPE_BUF);
    let (_open, dst) = gated_dst(Flag::default(), local_near);
    let (mut remote_far, piper) = connecting_piper(&watch, dst, PIPE_BUF);
    drive(piper, async {
        remote_far.write_all(&pattern(300, 4)).await.unwrap();
        remote_far.shutdown().await.unwrap();
        watch.ended.wait("join returned").await;
        let part = "connection closed before remote ready, 300 bytes dropped";
        assert!(
            watch.log.has_containing("INFO", part),
            "no line about discarded bytes"
        );
    })
    .await;
}

#[tokio::test]
async fn dst_error_destroys_with_that_error() {
    let watch = Watch::new(false);
    let (_remote_far, piper) =
        connecting_piper(&watch, ready(Err(io::Error::other("boom"))), PIPE_BUF);
    drive(piper, async {
        watch.ended.wait("join returned").await;
        let expected = Some((io::ErrorKind::Other, "boom".to_string()));
        assert_eq!(watch.destroyed.calls(), vec![expected]);
    })
    .await;
}
