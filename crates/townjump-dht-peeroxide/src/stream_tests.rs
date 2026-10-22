// Tests of the `stream` module: `wrap_connection` on a `PeerConnection` pair (UDX over 127.0.0.1, no DHT or JS)
// and two live tests on the JS testnet (`#[ignore]`). Stream: `shutdown` = END, drop before end of stream = DESTROY, error ≠ EOF.
use super::wrap_connection;
use libudx::{UdxAsyncStream, UdxRuntime, UdxSocket, UdxStream};
use peeroxide_dht::hyperdht::PeerConnection;
use peeroxide_dht::secret_stream::SecretStream;
use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use townjump_dht::DhtConnection;
use townjump_testkit::conformance::data::{
    assert_same, exchange, pattern, read_all, send_and_close,
};
use townjump_testkit::conformance::wait::{STREAM_WAIT, within};
use townjump_testkit::conformance::{half_close, stream_1mb};

// One helper file is included in three test files: a separate copy in each would be pointless.
#[allow(clippy::duplicate_mod)]
#[path = "support_tests.rs"]
mod support;

#[path = "stream_send_tests.rs"]
mod send;

const KEY_A: [u8; 32] = [0xa1; 32];
const KEY_B: [u8; 32] = [0xb2; 32];
const TX: [u8; 32] = [1; 32];
const RX: [u8; 32] = [2; 32];
const HANDSHAKE_HASH: [u8; 64] = [7; 64];
const MIB: usize = 1 << 20;

mod loopback_pair {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn reports_the_public_key_of_the_peer() {
        let pair = wrapped_pair().await;
        assert_eq!(pair.a.remote_public_key, KEY_B, "side A does not see key B");
        assert_eq!(pair.b.remote_public_key, KEY_A, "side B does not see key A");
    }

    mod bytes {
        use super::*;

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn written_on_one_side_arrive_on_the_other_both_ways() {
            let mut pair = wrapped_pair().await;
            assert_eq!(carry(&mut pair.a, &mut pair.b, b"ping").await, b"ping");
            assert_eq!(carry(&mut pair.b, &mut pair.a, b"pong").await, b"pong");
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn one_mebibyte_both_ways_arrives_equal() {
            let pair = wrapped_pair().await;
            exchange(
                pair.a.stream,
                pair.b.stream,
                MIB,
                "wrap: 1 MB in both directions",
            )
            .await;
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn small_reads_return_the_message_in_order() {
            let mut pair = wrapped_pair().await;
            let sent = pattern(64 * 1024, 3);
            let (_, got) = tokio::join!(
                write_flush(&mut pair.a, &sent),
                read_in_chunks(&mut pair.b, sent.len(), 1000)
            );
            assert_same(&got, &sent, "wrap: reading in 1000-byte chunks");
        }
    }

    mod shutdown {
        use super::*;

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn gives_eof_to_the_peer_who_can_still_answer() {
            let mut pair = wrapped_pair().await;
            send_and_close(&mut pair.a.stream, b"ping").await;
            assert_eq!(
                read_all(&mut pair.b.stream, "wrap: B until EOF").await,
                b"ping"
            );
            send_and_close(&mut pair.b.stream, b"pong").await;
            assert_eq!(
                read_all(&mut pair.a.stream, "wrap: A until EOF").await,
                b"pong"
            );
        }
    }

    mod drop_before_shutdown {
        use super::*;

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn is_an_error_for_the_peer_not_an_eof() {
            let mut pair = wrapped_pair().await;
            assert_eq!(carry(&mut pair.a, &mut pair.b, b"hello").await, b"hello");
            drop(pair.a);
            let mut rest = Vec::new();
            let read = within(
                STREAM_WAIT,
                "wrap: read after drop",
                pair.b.stream.read_to_end(&mut rest),
            );
            assert!(
                read.await.is_err(),
                "an abort (DESTROY) must not look like a clean EOF"
            );
        }
    }
}

mod live {
    use super::support::start_testnet;
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "needs the JS testnet: node and .tmp/npm-holesail"]
    async fn one_mebibyte_both_ways_through_the_dht_arrives_equal() {
        let testnet = start_testnet();
        stream_1mb(&testnet.factory).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "needs the JS testnet: node and .tmp/npm-holesail"]
    async fn half_close_through_the_dht_lets_the_server_answer() {
        let testnet = start_testnet();
        half_close(&testnet.factory).await;
    }
}

/// A wrapped connection pair; `_rt` keeps the UDX runtimes alive.
struct Wrapped {
    a: DhtConnection,
    b: DhtConnection,
    _rt: [UdxRuntime; 2],
}

async fn wrapped_pair() -> Wrapped {
    let (a, b, rt) = within(STREAM_WAIT, "UDX pair on 127.0.0.1", raw_pair()).await;
    Wrapped {
        a: wrap_connection(a),
        b: wrap_connection(b),
        _rt: rt,
    }
}

/// Two fork connections facing each other: UDX streams over loopback, with a `SecretStream` and a shared session on top.
async fn raw_pair() -> (PeerConnection, PeerConnection, [UdxRuntime; 2]) {
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

/// `bytes` go from `from` (write and flush) and are read at `to` for exactly their length.
async fn carry(from: &mut DhtConnection, to: &mut DhtConnection, bytes: &[u8]) -> Vec<u8> {
    let written = write_flush(from, bytes);
    let read = read_in_chunks(to, bytes.len(), bytes.len());
    tokio::join!(written, read).1
}

async fn write_flush(conn: &mut DhtConnection, bytes: &[u8]) {
    let write = async {
        conn.stream.write_all(bytes).await.expect("write_all");
        conn.stream.flush().await.expect("flush");
    };
    within(STREAM_WAIT, "wrap: write", write).await;
}

/// Reads exactly `total` bytes in chunks of at most `chunk`.
async fn read_in_chunks(conn: &mut DhtConnection, total: usize, chunk: usize) -> Vec<u8> {
    let read = async {
        let mut got = Vec::with_capacity(total);
        while got.len() < total {
            let mut buf = vec![0; chunk.min(total - got.len())];
            let n = conn.stream.read(&mut buf).await.expect("read");
            assert!(n > 0, "stream ended at {} of {total} bytes", got.len());
            got.extend_from_slice(&buf[..n]);
        }
        got
    };
    within(STREAM_WAIT, "wrap: read", read).await
}
