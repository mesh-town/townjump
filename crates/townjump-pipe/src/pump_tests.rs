// Tests of `pump`: a write does not wait for an ACK per chunk, and a tail stuck in the transport leaves without new data too.
// Stub writers repeat two properties of the UDX/SecretStream stream: `flush` waits for an ACK, a frame may sit in the transport.
use super::{LOCAL_TO_REMOTE, pump};
use crate::support::within;
use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::{AsyncWrite, AsyncWriteExt, DuplexStream, duplex};

const MIB: usize = 1 << 20;

/// What the writer has already put "on the wire".
#[derive(Clone, Default)]
struct Wire(Arc<Mutex<Vec<u8>>>);

impl Wire {
    fn put(&self, bytes: &[u8]) {
        self.0.lock().unwrap().extend_from_slice(bytes);
    }

    fn len(&self) -> usize {
        self.0.lock().unwrap().len()
    }

    async fn reaches(&self, what: &str, len: usize) {
        let arrived = async {
            while self.len() < len {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        };
        within(what, arrived).await;
    }
}

/// Accepts everything at once, but `flush` never completes: the ACK will not come.
struct NoAckWriter(Wire);

impl AsyncWrite for NoAckWriter {
    fn poll_write(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.0.put(buf);
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Pending
    }

    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

/// Like `SecretStream` with a full window: what was accepted sits in the writer and goes on the wire only with the next `poll_*`.
struct HoldingWriter {
    held: Vec<u8>,
    wire: Wire,
}

impl HoldingWriter {
    fn new(wire: &Wire) -> Self {
        Self {
            held: Vec::new(),
            wire: wire.clone(),
        }
    }

    fn release(&mut self) {
        self.wire.put(&self.held);
        self.held.clear();
    }
}

impl AsyncWrite for HoldingWriter {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.release();
        self.held.extend_from_slice(buf);
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.release();
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.poll_flush(cx)
    }
}

/// Runs `pump` over `writer` in its own task; `source` is the end where the test writes what `pump` reads.
fn spawn_pump<W: AsyncWrite + Unpin + Send + 'static>(
    writer: W,
) -> (DuplexStream, tokio::task::JoinHandle<()>) {
    let (source, reader) = duplex(64 * 1024);
    let task = tokio::spawn(async move {
        let _ = pump(reader, writer, LOCAL_TO_REMOTE, &None).await;
    });
    (source, task)
}

#[tokio::test]
async fn megabyte_passes_although_flush_never_completes() {
    let wire = Wire::default();
    let data = vec![7u8; MIB];
    let run = pump(
        data.as_slice(),
        NoAckWriter(wire.clone()),
        LOCAL_TO_REMOTE,
        &None,
    );
    within("pump over a writer without ACK", run).await.unwrap();
    assert_eq!(wire.len(), MIB, "not everything reached the wire");
}

#[tokio::test]
async fn idle_source_waiting_for_flush_does_not_stop_the_next_chunk() {
    let wire = Wire::default();
    let (mut source, _task) = spawn_pump(NoAckWriter(wire.clone()));
    for (i, part) in [b"aa".as_slice(), b"bbb", b"c"].into_iter().enumerate() {
        source.write_all(part).await.unwrap();
        wire.reaches("chunk after the source idle", [2, 5, 6][i])
            .await;
        // While the source is silent, `pump` waits for an ACK on `flush` that will not come; this must not stop reading.
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn tail_kept_by_the_writer_reaches_the_wire_while_the_source_is_silent() {
    let wire = Wire::default();
    let (mut source, _task) = spawn_pump(HoldingWriter::new(&wire));
    source.write_all(b"ping").await.unwrap();
    wire.reaches("tail without new data", 4).await;
}

#[tokio::test]
async fn source_end_releases_the_tail_and_closes_the_writer() {
    let wire = Wire::default();
    let (mut source, task) = spawn_pump(HoldingWriter::new(&wire));
    source.write_all(b"last").await.unwrap();
    source.shutdown().await.unwrap();
    within("source end", task).await.unwrap();
    assert_eq!(wire.len(), 4);
}
