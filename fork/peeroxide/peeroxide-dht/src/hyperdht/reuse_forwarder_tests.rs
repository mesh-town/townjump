// Datagram forwarder of the held-socket store; helpers are in `reuse_tests.rs`.
// Unpaused time: the forwarder waits on a real network, so the channel wait is a `timeout`.
// JS: hyperdht/lib/socket-pool.js

use super::*;

#[tokio::test]
async fn passes_dht_datagram_with_sender_and_socket() {
    // given
    let mut fx = setup();
    let (s, _lease) = fx.held().await;
    let r = fx.socket().await;
    // when
    fx.send(&r, &s, b"hello").await;
    // then
    let (addr, data, socket) = fx.next_forwarded().await;
    assert_eq!(addr, r.local_addr().await.expect("sender addr"));
    assert_eq!(data, b"hello".to_vec());
    assert!(
        socket.ptr_eq(&s),
        "the datagram must arrive with its held socket"
    );
}

#[tokio::test]
async fn skips_holepunch_probe() {
    // given
    let mut fx = setup();
    let (s, _lease) = fx.held().await;
    let r = fx.socket().await;
    // when: a 1-byte probe, then a DHT datagram
    fx.send(&r, &s, &[0]).await;
    sleep(Duration::from_millis(50)).await; // order of the two datagrams
    fx.send(&r, &s, b"hello").await;
    // then
    let (_, data, _) = fx.next_forwarded().await;
    assert_eq!(
        data,
        b"hello".to_vec(),
        "a 1-byte probe must not reach the node"
    );
    assert!(fx.nothing_forwarded_for(Duration::from_millis(100)).await);
}

#[tokio::test]
async fn passes_two_byte_datagram() {
    // given
    let mut fx = setup();
    let (s, _lease) = fx.held().await;
    let r = fx.socket().await;
    // when
    fx.send(&r, &s, &[7, 7]).await;
    // then
    let (_, data, _) = fx.next_forwarded().await;
    assert_eq!(
        data,
        vec![7, 7],
        "only datagrams of 1 byte or less are probes"
    );
}

#[tokio::test]
async fn second_hold_keeps_forwarding_once() {
    // given: the same socket held twice
    let mut fx = setup();
    let (s, _first) = fx.held().await;
    let _second = fx.reuse.hold(&s).expect("second hold");
    let r = fx.socket().await;
    // when
    fx.send(&r, &s, b"hello").await;
    // then
    let (_, data, _) = fx.next_forwarded().await;
    assert_eq!(data, b"hello".to_vec());
    assert!(
        fx.nothing_forwarded_for(Duration::from_millis(100)).await,
        "one datagram, one forward"
    );
}

#[tokio::test]
async fn stops_after_clear() {
    // given: the path works (control), then clear
    let mut fx = setup();
    let (s, _lease) = fx.held().await;
    let r = fx.socket().await;
    fx.send(&r, &s, b"hello").await;
    fx.next_forwarded().await;
    // when
    fx.reuse.clear();
    fx.send(&r, &s, b"hello").await;
    // then
    assert!(
        fx.nothing_forwarded_for(Duration::from_millis(300)).await,
        "a cleared store must not forward"
    );
}
