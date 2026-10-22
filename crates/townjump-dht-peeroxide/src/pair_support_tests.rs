// Network-free test helper: a pair of fork connections over UDX on 127.0.0.1 (no DHT or JS) and byte exchange over it.
// Included via `#[path]` from slot_tests, tunnels_tests and node_suspend_tests; each needs its own part.
#![allow(dead_code)]
use libudx::{UdxAsyncStream, UdxRuntime, UdxSocket, UdxStream};
use peeroxide_dht::hyperdht::PeerConnection;
use peeroxide_dht::secret_stream::SecretStream;
use std::future::poll_fn;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf, ReadHalf, WriteHalf};
use tokio::sync::oneshot;
use townjump_dht::Duplex;
use townjump_testkit::conformance::wait::{STREAM_WAIT, within};

/// Public key of side A: side B sees it (`b.remote_public_key`).
pub const KEY_A: [u8; 32] = [0xa1; 32];
/// Public key of side B: side A sees it (`a.remote_public_key`).
pub const KEY_B: [u8; 32] = [0xb2; 32];
const TX: [u8; 32] = [1; 32];
const RX: [u8; 32] = [2; 32];
const HANDSHAKE_HASH: [u8; 64] = [7; 64];
const CHUNK: usize = 64 * 1024;
/// Write cap until it stalls: without the remote peer replying, the window is not infinite.
const WRITE_CAP: usize = 128 << 20;

/// Two fork connections facing each other: UDX streams over loopback, with a `SecretStream` and a shared session on top.
/// UDX runtimes must be kept alive while the pair is needed.
pub async fn raw_pair() -> (PeerConnection, PeerConnection, [UdxRuntime; 2]) {
    let rt = [runtime(), runtime()];
    let (sock_a, addr_a) = bound_socket(&rt[0]).await;
    let (sock_b, addr_b) = bound_socket(&rt[1]).await;
    let stream_a = rt[0].create_stream(1).await.expect("stream A");
    let stream_b = rt[1].create_stream(2).await.expect("stream B");
    stream_a
        .connect(&sock_a, 2, addr_b)
        .await
        .expect("connect A");
    stream_b
        .connect(&sock_b, 1, addr_a)
        .await
        .expect("connect B");
    let (ss_a, ss_b) = tokio::join!(
        session(true, stream_a, (TX, RX), KEY_B),
        session(false, stream_b, (RX, TX), KEY_A)
    );
    let a = PeerConnection::new(ss_a, KEY_B, sock_a, None);
    let b = PeerConnection::new(ss_b, KEY_A, sock_b, None);
    (a, b, rt)
}

/// `raw_pair` capped by the `STREAM_WAIT` wait.
pub async fn pair_within() -> (PeerConnection, PeerConnection, [UdxRuntime; 2]) {
    within(STREAM_WAIT, "UDX pair on 127.0.0.1", raw_pair()).await
}

fn runtime() -> UdxRuntime {
    UdxRuntime::new().expect("UDX runtime")
}

async fn bound_socket(rt: &UdxRuntime) -> (UdxSocket, SocketAddr) {
    let socket = rt.create_socket().await.expect("UDX socket");
    socket
        .bind("127.0.0.1:0".parse().expect("address"))
        .await
        .expect("bind");
    let addr = socket.local_addr().await.expect("local_addr");
    (socket, addr)
}

async fn session(
    initiator: bool,
    stream: UdxStream,
    (tx, rx): ([u8; 32], [u8; 32]),
    remote: [u8; 32],
) -> SecretStream<UdxAsyncStream> {
    let raw = stream.into_async_stream();
    let opened = SecretStream::from_session(initiator, raw, tx, rx, HANDSHAKE_HASH, remote);
    opened.await.expect("from_session")
}

/// `bytes` go out from `from` (write and flush) and are read at `to` for exactly their length.
pub async fn carry(from: &mut Box<dyn Duplex>, to: &mut Box<dyn Duplex>, bytes: &[u8]) -> Vec<u8> {
    let write = async {
        from.write_all(bytes).await.expect("write_all");
        from.flush().await.expect("flush");
    };
    let read = async {
        let mut got = vec![0; bytes.len()];
        to.read_exact(&mut got).await.expect("read_exact");
        got
    };
    within(STREAM_WAIT, "byte transfer", async {
        tokio::join!(write, read).1
    })
    .await
}

/// Reads to the end of the stream (`EOF` or an error) for at most `STREAM_WAIT`; `Ok` — clean EOF, `Err` — abort.
pub async fn read_to_end_within(
    stream: &mut Box<dyn Duplex>,
    what: &str,
) -> std::io::Result<Vec<u8>> {
    let mut rest = Vec::new();
    let read = within(STREAM_WAIT, what, stream.read_to_end(&mut rest)).await;
    read.map(|_| rest)
}

/// Reads until it parks in `Pending`: then reports to `parked` (the slot's Waker is recorded by now) and keeps waiting.
/// The result is how the read ended.
pub async fn read_until_parked(
    mut half: ReadHalf<Box<dyn Duplex>>,
    parked: oneshot::Sender<()>,
) -> io::Result<usize> {
    let mut parked = Some(parked);
    let mut bytes = [0u8; 16];
    poll_fn(|cx| {
        let mut buf = ReadBuf::new(&mut bytes);
        let polled = AsyncRead::poll_read(Pin::new(&mut half), cx, &mut buf);
        if polled.is_pending() {
            notify(&mut parked, ());
        }
        polled.map_ok(|()| buf.filled().len())
    })
    .await
}

/// Writes in chunks until the write stalls: then reports the amount sent to `parked` and keeps waiting.
/// The result is the error the write ended with; if it reached `WRITE_CAP` and did not stall, `parked` is closed without a reply.
pub async fn write_until_parked(
    mut half: WriteHalf<Box<dyn Duplex>>,
    parked: oneshot::Sender<usize>,
) -> io::Result<()> {
    let mut parked = Some(parked);
    let chunk = vec![0x5a; CHUNK];
    let mut sent = 0;
    while sent < WRITE_CAP {
        let written = poll_fn(|cx| {
            let polled = AsyncWrite::poll_write(Pin::new(&mut half), cx, &chunk);
            if polled.is_pending() {
                notify(&mut parked, sent);
            }
            polled
        })
        .await?;
        sent += written;
    }
    Ok(())
}

fn notify<T>(slot: &mut Option<oneshot::Sender<T>>, value: T) {
    if let Some(sender) = slot.take() {
        let _ = sender.send(value);
    }
}
