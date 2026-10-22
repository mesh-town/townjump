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
