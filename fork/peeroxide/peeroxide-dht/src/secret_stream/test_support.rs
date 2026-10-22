//! Test doubles: an in-memory transport that counts writes and flushes, can fail reads and can hold
//! `flush` pending (an unacknowledged UDX write), and a ready pair of keyed `SecretStream`s over it.
// JS: @hyperswarm/secret-stream/index.js

use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use tokio::io::{AsyncRead, AsyncWrite, DuplexStream, ReadBuf};

use super::session::SecretStream;

/// Observation and fault-injection switches of one [`TestIo`].
#[derive(Default)]
pub(super) struct Probe {
    /// Number of `poll_write` calls that reached the transport.
    pub writes: AtomicUsize,
    /// While set, every read fails with `ConnectionReset`.
    pub fail_reads: AtomicBool,
    /// Number of `poll_flush` calls (re-polls of a held flush included).
    pub flushes: AtomicUsize,
    hold_flush: AtomicBool,
    flush_waker: Mutex<Option<Waker>>,
}

impl Probe {
    /// From now on `poll_flush` stays pending, like UDX `flush` before the peer's ACK has arrived.
    pub(super) fn hold_flush(&self) {
        let _slot = self.flush_waker.lock().expect("flush waker lock");
        self.hold_flush.store(true, Ordering::SeqCst);
    }

    /// The ACK arrives: pending flushes complete.
    pub(super) fn release_flush(&self) {
        let mut slot = self.flush_waker.lock().expect("flush waker lock");
        self.hold_flush.store(false, Ordering::SeqCst);
        if let Some(waker) = slot.take() {
            waker.wake();
        }
    }
}

/// `DuplexStream` passthrough with a [`Probe`].
pub(super) struct TestIo {
    inner: DuplexStream,
    probe: Arc<Probe>,
}

/// One end of a [`TestIo`] pair with its probe.
pub(super) type Side = (TestIo, Arc<Probe>);

impl TestIo {
    fn new(inner: DuplexStream) -> Side {
        let probe = Arc::new(Probe::default());
        let io = Self {
            inner,
            probe: Arc::clone(&probe),
        };
        (io, probe)
    }

    /// Two connected ends with `capacity` bytes of buffer per direction.
    pub(super) fn pair(capacity: usize) -> (Side, Side) {
        let (a, b) = tokio::io::duplex(capacity);
        (Self::new(a), Self::new(b))
    }
}

impl AsyncRead for TestIo {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.probe.fail_reads.load(Ordering::SeqCst) {
            return Poll::Ready(Err(io::ErrorKind::ConnectionReset.into()));
        }
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for TestIo {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.probe.writes.fetch_add(1, Ordering::SeqCst);
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.probe.flushes.fetch_add(1, Ordering::SeqCst);
        let mut slot = self.probe.flush_waker.lock().expect("flush waker lock");
        if self.probe.hold_flush.load(Ordering::SeqCst) {
            *slot = Some(cx.waker().clone());
            return Poll::Pending;
        }
        drop(slot);
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// One `SecretStream` end of [`session_pair`] with its probe.
pub(super) type SecretSide = (SecretStream<TestIo>, Arc<Probe>);

/// Two `SecretStream`s over [`TestIo`], keyed as in `from_session_roundtrip`.
pub(super) async fn session_pair(capacity: usize) -> (SecretSide, SecretSide) {
    session_pair_over(TestIo::pair(capacity)).await
}

/// [`session_pair`] whose flushes are held on both ends before the header exchange.
pub(super) async fn session_pair_held_flush(capacity: usize) -> (SecretSide, SecretSide) {
    let pair = TestIo::pair(capacity);
    pair.0.1.hold_flush();
    pair.1.1.hold_flush();
    session_pair_over(pair).await
}

async fn session_pair_over(
    ((a_io, a_probe), (b_io, b_probe)): (Side, Side),
) -> (SecretSide, SecretSide) {
    let (tx, rx, hash, pk) = ([0x11u8; 32], [0x22u8; 32], [0xABu8; 64], [0xCCu8; 32]);
    let (a, b) = tokio::join!(
        SecretStream::from_session(true, a_io, tx, rx, hash, pk),
        SecretStream::from_session(false, b_io, rx, tx, hash, pk),
    );
    let a = a.expect("side A from_session failed");
    let b = b.expect("side B from_session failed");
    ((a, a_probe), (b, b_probe))
}
