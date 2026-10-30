//! Tests for [`crate::socket_pool`].

use super::*;
use crate::hyperdht_messages::{
    FIREWALL_CONSISTENT, FIREWALL_OPEN, FIREWALL_RANDOM, FIREWALL_UNKNOWN,
};

#[test]
fn random_port_in_range() {
    for _ in 0..1000 {
        let p = random_port();
        assert!(p >= 1000);
    }
}

#[test]
fn coerce_open_to_consistent() {
    assert_eq!(coerce_firewall(FIREWALL_OPEN), FIREWALL_CONSISTENT);
}

#[test]
fn coerce_consistent_unchanged() {
    assert_eq!(coerce_firewall(FIREWALL_CONSISTENT), FIREWALL_CONSISTENT);
}

#[test]
fn coerce_random_unchanged() {
    assert_eq!(coerce_firewall(FIREWALL_RANDOM), FIREWALL_RANDOM);
}

#[test]
fn coerce_unknown_unchanged() {
    assert_eq!(coerce_firewall(FIREWALL_UNKNOWN), FIREWALL_UNKNOWN);
}

#[test]
fn demux_routes_one_byte_to_holepunch() {
    let buf = [0u8; 1];
    assert_eq!(classify_inbound(&buf), InboundClass::Holepunch);
}

#[test]
fn demux_drops_udx_framed_packet() {
    let mut buf = vec![0u8; 25];
    buf[0] = 0xFF;
    assert_eq!(
        classify_inbound(&buf),
        InboundClass::UdxFrame,
        "25-byte 0xFF-prefix packet must classify as UdxFrame",
    );
}

#[test]
fn demux_routes_dht_response_to_reply_lane() {
    use crate::messages::{Ipv4Peer, Response, encode_response_to_bytes};
    let resp = Response {
        tid: 1,
        to: Ipv4Peer {
            host: "127.0.0.1".into(),
            port: 9000,
        },
        id: None,
        token: None,
        closer_nodes: vec![],
        error: 0,
        value: None,
    };
    let buf = encode_response_to_bytes(&resp).unwrap();
    assert_eq!(
        classify_inbound(&buf),
        InboundClass::DhtResponse,
        "valid DHT Response bytes must classify as DhtResponse",
    );
}

#[tokio::test]
async fn low_ttl_holepunch_restores_socket_ttl() {
    let rt = libudx::UdxRuntime::new().expect("runtime");
    let pool = SocketPool::new("127.0.0.1".to_string());
    let a = pool.acquire(&rt).await.expect("acquire a");
    let mut b = pool.acquire(&rt).await.expect("acquire b");
    let mut rx_b = b.take_holepunch_rx().expect("holepunch rx");
    let addr_b = b.socket.local_addr().await.expect("addr b");

    a.socket.set_ttl(64).expect("set_ttl");
    a.send_holepunch(addr_b, true).expect("send_holepunch");
    assert_eq!(a.socket.ttl().expect("ttl"), 64);

    let ev = tokio::time::timeout(std::time::Duration::from_secs(2), rx_b.recv())
        .await
        .expect("holepunch timed out")
        .expect("channel closed");
    assert_eq!(
        ev.addr.port(),
        a.socket.local_addr().await.expect("addr a").port()
    );
}

mod udx_frame_sink {
    use super::*;
    use libudx::UdxRuntime;

    mod pool_with_a_sink {
        use super::*;

        #[tokio::test]
        async fn udx_frame_reaches_the_sink_with_its_socket() {
            // given
            let rt = UdxRuntime::new().expect("runtime");
            let (pool, mut sink) = sink_pool();
            let served = pool.acquire(&rt).await.expect("acquire");
            let served_addr = served.socket.local_addr().await.expect("served addr");

            // when
            let client = send_real_frame(&rt, served_addr).await;

            // then
            let seen = recv_within(&mut sink).await.expect("no frame in the sink");
            let seen_port = seen
                .socket
                .local_addr()
                .await
                .expect("sink socket addr")
                .port();
            assert_eq!(
                seen_port,
                served_addr.port(),
                "the sink must carry the receiving socket"
            );
            assert_eq!(
                seen.addr, client.addr,
                "the sink must carry the sender's address"
            );
        }

        #[tokio::test]
        async fn probe_still_reaches_holepunch_rx() {
            // given
            let rt = UdxRuntime::new().expect("runtime");
            let (pool, mut sink) = sink_pool();
            let mut served = pool.acquire(&rt).await.expect("acquire");
            let mut hp_rx = served.take_holepunch_rx().expect("holepunch rx");
            let served_addr = served.socket.local_addr().await.expect("served addr");
            let client = bound(&rt).await;

            // when
            client.send_to(&[0], served_addr).expect("send probe");

            // then
            let ev = recv_within(&mut hp_rx).await.expect("no holepunch event");
            assert_eq!(ev.addr, client.local_addr().await.expect("client addr"));
            assert!(
                sink.try_recv().is_err(),
                "a probe is not a UDX frame: the sink stays empty"
            );
        }

        #[tokio::test]
        async fn dropped_sink_releases_the_socket() {
            // given
            let rt = UdxRuntime::new().expect("runtime");
            let (pool, sink) = sink_pool();
            let served = pool.acquire(&rt).await.expect("acquire");
            let port = served
                .socket
                .local_addr()
                .await
                .expect("served addr")
                .port();

            // when
            drop(sink);
            drop(served);

            // then
            assert!(
                port_frees_up(port).await,
                "port {port} is still held 2 s after sink and ref are gone"
            );
            drop(pool);
        }
    }

    mod pool_without_a_sink {
        use super::*;

        #[tokio::test]
        async fn udx_frame_without_sink_is_dropped() {
            // given
            let rt = UdxRuntime::new().expect("runtime");
            let pool = SocketPool::new("127.0.0.1".to_string());
            let mut served = pool.acquire(&rt).await.expect("acquire");
            let mut hp_rx = served.take_holepunch_rx().expect("holepunch rx");
            let served_addr = served.socket.local_addr().await.expect("served addr");
            let client = send_real_frame(&rt, served_addr).await;

            // when
            client
                .socket
                .send_to(&[0], served_addr)
                .expect("send probe");

            // then
            let ev = recv_within(&mut hp_rx).await;
            assert!(
                ev.is_some(),
                "the probe after a dropped frame must still be routed"
            );
        }
    }
}

struct Client {
    socket: libudx::UdxSocket,
    addr: std::net::SocketAddr,
}

fn sink_pool() -> (SocketPool, mpsc::UnboundedReceiver<UdxFrameSeen>) {
    let (tx, rx) = mpsc::unbounded_channel();
    (
        SocketPool::new("127.0.0.1".to_string()).with_udx_frames(tx),
        rx,
    )
}

pub(super) async fn bound(rt: &libudx::UdxRuntime) -> libudx::UdxSocket {
    let socket = rt.create_socket().await.expect("socket");
    socket
        .bind("127.0.0.1:0".parse().expect("addr"))
        .await
        .expect("bind");
    socket
}

/// Sends a real UDX data frame to `to` (stream id 12345 is unknown there). The write waits for an
/// ACK that never comes, so it runs in a task.
async fn send_real_frame(rt: &libudx::UdxRuntime, to: std::net::SocketAddr) -> Client {
    let socket = bound(rt).await;
    let addr = socket.local_addr().await.expect("client addr");
    let stream = rt.create_stream(9).await.expect("stream");
    stream.connect(&socket, 12345, to).await.expect("connect");
    tokio::spawn(async move {
        let _ = stream.write(b"x").await;
    });
    Client { socket, addr }
}

async fn recv_within<T>(rx: &mut mpsc::UnboundedReceiver<T>) -> Option<T> {
    tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
        .await
        .ok()
        .flatten()
}

/// Binding the port succeeds within 2 s (retried every 100 ms).
pub(super) async fn port_frees_up(port: u16) -> bool {
    for _ in 0..20 {
        if tokio::net::UdpSocket::bind(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    false
}
