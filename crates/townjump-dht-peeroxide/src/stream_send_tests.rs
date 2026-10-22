// End-to-end send path: TCP → `conn_piper` → `PeerStream` (SecretStream, UDX over loopback) → `conn_piper` → TCP.
// Guards against `flush` waiting for an ACK after every chunk, which would cap throughput at one 16 KiB chunk per RTT.
use super::*;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use townjump_dht::Duplex;
use townjump_pipe::{PipeStats, PiperOpts, conn_piper};

const VOLUME: usize = 8 * MIB;
/// Chunk of `pump` (townjump-pipe): this many writes go out per volume.
const CHUNKS: usize = VOLUME / (16 * 1024);
/// Below the keepalive (5 s): a tail stuck in the transport would survive to the deadline only thanks to it.
const TAIL_WAIT: Duration = Duration::from_secs(4);

/// Sender stream counting the `flush` calls that had to wait (the ACK did not arrive at once).
struct WaitCounting {
    inner: Box<dyn Duplex>,
    waited: Arc<AtomicUsize>,
}

impl AsyncRead for WaitCounting {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for WaitCounting {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let polled = Pin::new(&mut self.inner).poll_flush(cx);
        if polled.is_pending() {
            self.waited.fetch_add(1, SeqCst);
        }
        polled
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// A source application and a sink application over TCP, with two `conn_piper`s and a tunnel between them.
struct Piped {
    source: TcpStream,
    sink: TcpStream,
    waited: Arc<AtomicUsize>,
    _rt: [UdxRuntime; 2],
}

async fn tcp_pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let (client, accepted) = tokio::join!(TcpStream::connect(addr), listener.accept());
    (client.expect("connect"), accepted.expect("accept").0)
}

async fn piped() -> Piped {
    let pair = wrapped_pair().await;
    let (source, near) = tcp_pair().await;
    let (far, sink) = tcp_pair().await;
    let waited = Arc::new(AtomicUsize::new(0));
    let sender = WaitCounting {
        inner: pair.a.stream,
        waited: waited.clone(),
    };
    let stats = Arc::new(PipeStats::default());
    tokio::spawn(conn_piper(
        sender,
        near,
        PiperOpts::default(),
        stats.clone(),
    ));
    tokio::spawn(conn_piper(pair.b.stream, far, PiperOpts::default(), stats));
    Piped {
        source,
        sink,
        waited,
        _rt: pair._rt,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tunnel_send_does_not_wait_for_ack_per_chunk() {
    let Piped {
        mut source,
        mut sink,
        waited,
        _rt,
    } = piped().await;
    let sent = pattern(VOLUME, 5);
    let reader = tokio::spawn(async move {
        let mut got = vec![0; VOLUME];
        sink.read_exact(&mut got).await.expect("read_exact");
        got
    });
    within(STREAM_WAIT, "send into the tunnel", source.write_all(&sent))
        .await
        .expect("write_all");
    // The source is open and silent: the rest must arrive by itself, without new writes and without keepalive.
    let got = within(TAIL_WAIT, "tail after the end of writing", reader)
        .await
        .expect("read task");
    assert_same(&got, &sent, "tunnel: 8 MiB through conn_piper");
    let waited = waited.load(SeqCst);
    assert!(
        waited <= CHUNKS / 8,
        "flush waited for an ACK {waited} times out of {CHUNKS} chunks: the write waits for an ACK per chunk again"
    );
}
