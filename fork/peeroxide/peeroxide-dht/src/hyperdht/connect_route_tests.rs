//! `Via::open`: the client stream is in the socket's table before any request is sent.
// JS: hyperdht/lib/connect.js:86

use std::net::SocketAddr;
use std::time::Duration;

use libudx::UdxRuntime;
use tokio::net::UdpSocket;

use super::*;

const QUIET: Duration = Duration::from_millis(200);
const WAIT: Duration = Duration::from_secs(1);

/// DATA packet (UDX header, 20 bytes LE) to stream `remote_id`.
fn data_packet(remote_id: u32, payload: &[u8]) -> Vec<u8> {
    // magic 0xff, version 0x01, type 0x01 (DATA), data offset 0x00
    let mut packet = vec![0xff, 0x01, 0x01, 0x00];
    packet.extend_from_slice(&remote_id.to_le_bytes());
    packet.extend_from_slice(&4_194_304u32.to_le_bytes());
    packet.extend_from_slice(&[0u8; 8]);
    packet.extend_from_slice(payload);
    packet
}

#[tokio::test]
async fn open_registers_the_stream_before_any_request() {
    // given: a bound socket with receive and a raw UDP peer
    let runtime = UdxRuntime::new().expect("runtime");
    let socket = runtime.create_socket().await.expect("socket");
    socket
        .bind("127.0.0.1:0".parse().expect("addr"))
        .await
        .expect("bind");
    let addr = socket.local_addr().await.expect("local addr");
    let mut fallback = socket.recv_start().expect("recv_start");
    let raw = UdpSocket::bind("127.0.0.1:0").await.expect("raw");
    let raw_addr: SocketAddr = raw.local_addr().expect("raw addr");
    let mut via = Via::open(&runtime, &socket).await.expect("open");
    // when: the peer sends DATA to the stream id, then the stream is connected
    let id = via.stream.local_id();
    raw.send_to(&data_packet(id, b"early"), addr)
        .await
        .expect("send");
    let none = tokio::time::timeout(QUIET, fallback.recv()).await;
    assert!(none.is_err(), "packet went to the socket fallback channel");
    via.stream
        .connect(&socket, 1, raw_addr)
        .await
        .expect("connect");
    // then: the packet is readable
    let got = tokio::time::timeout(WAIT, via.stream.read()).await;
    let got = got.expect("read timeout").expect("read").expect("eof");
    assert_eq!(got, b"early");
}
