#![deny(clippy::all)]

//! Acceptance test: `SecretStream` as `AsyncRead`/`AsyncWrite` over a real pair
//! of UDX streams (two runtimes, loopback sockets).
//!
//! The pair checks what the in-memory `DuplexStream` tests cannot: writes do
//! not wait for ACK, a writer that ends (FIN, then DESTROY) still gives the
//! lagging reader a clean EOF, and dropping a live stream resets the peer.
//! JS: `@hyperswarm/secret-stream/index.js`, `udx-native/lib/stream.js`.
//!
//! Trait calls use the full path (`AsyncReadExt::read(&mut b, ..)`): the
//! inherent `read`/`write`/`shutdown` of `SecretStream` shadow the trait ones.

use std::future::Future;
use std::io::ErrorKind;
use std::net::SocketAddr;
use std::time::Duration;

use libudx::{UdxAsyncStream, UdxRuntime, UdxSocket};
use peeroxide_dht::noise;
use peeroxide_dht::secret_stream::SecretStream;
use rand::Rng;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const LOCALHOST: &str = "127.0.0.1:0";
const TIMEOUT: Duration = Duration::from_secs(15);
const MEGABYTE: usize = 1024 * 1024;
const CHUNK: usize = 7777;

/// Everything that must outlive the streams: runtimes and sockets. Tests bind
/// it as `_keep` (a bare `_` or `..` would drop it at the end of the `let`).
struct Keep {
    _runtimes: (UdxRuntime, UdxRuntime),
    _sockets: (UdxSocket, UdxSocket),
}

type Secret = SecretStream<UdxAsyncStream>;

fn addr() -> SocketAddr {
    LOCALHOST.parse().unwrap()
}

async fn bound_socket(rt: &UdxRuntime) -> (UdxSocket, SocketAddr) {
    let socket = rt.create_socket().await.expect("create_socket");
    socket.bind(addr()).await.expect("bind");
    let local = socket.local_addr().await.expect("local_addr");
    (socket, local)
}

async fn with_timeout<T>(what: &str, fut: impl Future<Output = T>) -> T {
    match tokio::time::timeout(TIMEOUT, fut).await {
        Ok(v) => v,
        Err(_) => panic!("{what}: timed out after {TIMEOUT:?}"),
    }
}

/// Two UDX streams (local ids 1 and 2) on separate runtimes, joined by a
/// Noise XX handshake into a `SecretStream` pair (`a` initiator, `b` responder).
async fn secret_pair() -> (Keep, Secret, Secret) {
    let rt_a = UdxRuntime::new().expect("runtime a");
    let rt_b = UdxRuntime::new().expect("runtime b");
    let (sock_a, addr_a) = bound_socket(&rt_a).await;
    let (sock_b, addr_b) = bound_socket(&rt_b).await;
    let stream_a = rt_a.create_stream(1).await.expect("stream a");
    let stream_b = rt_b.create_stream(2).await.expect("stream b");
    stream_a
        .connect(&sock_a, 2, addr_b)
        .await
        .expect("connect a");
    stream_b
        .connect(&sock_b, 1, addr_a)
        .await
        .expect("connect b");

    let (raw_a, raw_b) = (stream_a.into_async_stream(), stream_b.into_async_stream());
    let handshake = async {
        tokio::join!(
            SecretStream::new(true, raw_a, noise::generate_keypair()),
            SecretStream::new(false, raw_b, noise::generate_keypair()),
        )
    };
    let (a, b) = with_timeout("handshake", handshake).await;
    let keep = Keep {
        _runtimes: (rt_a, rt_b),
        _sockets: (sock_a, sock_b),
    };
    (keep, a.expect("handshake a"), b.expect("handshake b"))
}

/// 1 MiB of random data goes A -> B through `copy`; the writer task ends by
/// dropping the stream (FIN, then DESTROY), so B is a lagging reader that must
/// still get every byte and a clean EOF (`Ok(0)`), not `ConnectionReset`.
#[tokio::test]
async fn megabyte_over_udx_read_in_chunks() {
    let (_keep, a, mut b) = secret_pair().await;
    let mut data = vec![0u8; MEGABYTE];
    rand::rng().fill(&mut data[..]);

    let sent = data.clone();
    let writer = tokio::spawn(async move {
        let mut a = a;
        let copied = tokio::io::copy(&mut &sent[..], &mut a).await.expect("copy");
        assert_eq!(copied, sent.len() as u64);
        AsyncWriteExt::shutdown(&mut a).await.expect("shutdown");
    });

    let mut received = Vec::with_capacity(MEGABYTE);
    let mut buf = vec![0u8; CHUNK];
    let read_all = async {
        loop {
            let n = AsyncReadExt::read(&mut b, &mut buf).await.expect("read");
            if n == 0 {
                break;
            }
            received.extend_from_slice(&buf[..n]);
        }
    };
    with_timeout("read 1 MiB", read_all).await;
    writer.await.expect("writer task");

    assert_eq!(received.len(), data.len());
    assert!(received == data, "received bytes differ from sent bytes");
}

/// A keepalive frame (empty message) is consumed silently: the next `read`
/// returns the data message that follows it.
#[tokio::test]
async fn keepalive_over_udx_is_skipped() {
    let (_keep, mut a, mut b) = secret_pair().await;

    with_timeout("keepalive", async {
        a.write(&[]).await.expect("keepalive");
        AsyncWriteExt::write_all(&mut a, b"abc")
            .await
            .expect("write_all");
        AsyncWriteExt::flush(&mut a).await.expect("flush");
        let msg = b.read().await.expect("read");
        assert_eq!(msg.as_deref(), Some(&b"abc"[..]));
    })
    .await;
}

/// Dropping a live stream (no `shutdown`) sends DESTROY: the peer reads the
/// delivered byte, then gets `ConnectionReset`, twice, and never a plain EOF.
#[tokio::test]
async fn drop_over_udx_resets_peer() {
    let (_keep, mut a, mut b) = secret_pair().await;

    with_timeout("drop resets peer", async {
        AsyncWriteExt::write_all(&mut a, b"x")
            .await
            .expect("write_all");
        AsyncWriteExt::flush(&mut a).await.expect("flush");
        drop(a);

        let mut buf = [0u8; 16];
        let n = AsyncReadExt::read(&mut b, &mut buf).await.expect("read x");
        assert_eq!(&buf[..n], b"x");
        for _ in 0..2 {
            let err = AsyncReadExt::read(&mut b, &mut buf)
                .await
                .expect_err("peer drop must not look like EOF");
            assert_eq!(err.kind(), ErrorKind::ConnectionReset, "got {err:?}");
        }
    })
    .await;
}
