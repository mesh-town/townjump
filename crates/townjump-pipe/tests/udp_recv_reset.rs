#![cfg(windows)]
// Windows regression tests: a send to a closed UDP port makes the next receive fail with WSAECONNRESET (10054).
// The receive loops of the framed UDP server and proxy must skip that echo instead of ending. Public API only.
use std::future::Future;
use std::io;
use std::net::{SocketAddr, UdpSocket as StdUdp};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream, duplex};
use tokio::net::UdpSocket;
use tokio::sync::mpsc::unbounded_channel;
use tokio::time::{sleep, timeout};
use townjump_logger::{Logger, SharedOut};
use townjump_pipe::{
    FrameDecoder, PipeStats, create_udp_framed_proxy, encode_frame, pipe_udp_framed_server,
};

const STEP: Duration = Duration::from_secs(5);
const SETTLE: Duration = Duration::from_millis(300);

fn logger() -> Logger {
    let out: SharedOut = Arc::new(Mutex::new(io::sink()));
    Logger::new(Some(0.0), out.clone(), out)
}

async fn within<F: Future>(what: &str, f: F) -> F::Output {
    match timeout(STEP, f).await {
        Ok(value) => value,
        Err(_) => panic!("timed out after 5 s: {what}"),
    }
}

/// An address nobody listens on: bind, remember, close.
fn dead_addr() -> SocketAddr {
    let socket = StdUdp::bind("127.0.0.1:0").unwrap();
    socket.local_addr().unwrap()
}

async fn read_frame(stream: &mut DuplexStream) -> Vec<u8> {
    let mut decoder = FrameDecoder::default();
    let mut byte = [0u8; 1];
    loop {
        stream.read_exact(&mut byte).await.unwrap();
        if let Some(body) = decoder.push(&byte).pop() {
            return body;
        }
    }
}

async fn recv(socket: &UdpSocket) -> (Vec<u8>, SocketAddr) {
    let mut buf = vec![0u8; 2048];
    let (n, from) = socket.recv_from(&mut buf).await.unwrap();
    buf.truncate(n);
    (buf, from)
}

/// Sends `frames` datagrams to the closed port, then opens the service there and checks the round trip.
async fn server_scenario(frames: usize) {
    let dead = dead_addr();
    let (mut far, near) = duplex(64 * 1024);
    let stats = Arc::new(PipeStats::default());
    let server = tokio::spawn(pipe_udp_framed_server(
        near,
        ("127.0.0.1".to_string(), dead.port()),
        logger(),
        stats,
    ));
    for _ in 0..frames {
        far.write_all(&encode_frame(b"one")).await.unwrap();
    }
    sleep(SETTLE).await;
    let service = UdpSocket::bind(dead).await.unwrap();
    far.write_all(&encode_frame(b"two")).await.unwrap();
    let (got, from) = within("service gets `two`", recv(&service)).await;
    assert_eq!(got, b"two");
    service.send_to(b"reply", from).await.unwrap();
    let reply = within("stream gets `reply`", read_frame(&mut far)).await;
    assert_eq!(reply, b"reply");
    assert!(!server.is_finished(), "server task ended");
    server.abort();
}

#[tokio::test]
async fn server_survives_send_to_closed_local_port() {
    server_scenario(1).await;
}

#[tokio::test]
async fn server_survives_burst_to_closed_local_port() {
    server_scenario(300).await;
}

#[tokio::test]
async fn proxy_survives_reply_to_closed_client_port() {
    let (streams_tx, mut streams) = unbounded_channel::<DuplexStream>();
    let connect = move || {
        let (far, near) = duplex(64 * 1024);
        let _ = streams_tx.send(far);
        async move { Ok::<_, io::Error>(near) }
    };
    let proxy = create_udp_framed_proxy(("127.0.0.1".to_string(), 0), connect, logger())
        .await
        .unwrap();
    let target = proxy.local_addr;

    let client_a = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    client_a.send_to(b"hello", target).await.unwrap();
    let mut stream_a = within("stream of A", streams.recv()).await.unwrap();
    assert_eq!(
        within("frame `hello`", read_frame(&mut stream_a)).await,
        b"hello"
    );
    drop(client_a);

    stream_a.write_all(&encode_frame(b"late")).await.unwrap();
    sleep(SETTLE).await;
    assert!(!proxy.task.is_finished(), "proxy receive loop ended");

    let client_b = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    client_b.send_to(b"world", target).await.unwrap();
    let mut stream_b = within("stream of B", streams.recv()).await.unwrap();
    assert_eq!(
        within("frame `world`", read_frame(&mut stream_b)).await,
        b"world"
    );
    stream_b.write_all(&encode_frame(b"pong")).await.unwrap();
    let (got, _) = within("B gets `pong`", recv(&client_b)).await;
    assert_eq!(got, b"pong");
    proxy.task.abort();
}
