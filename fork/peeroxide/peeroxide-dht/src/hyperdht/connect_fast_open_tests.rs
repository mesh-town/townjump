//! Tests for [`super`] (client fast mode).

use std::time::Duration;

use tokio::sync::mpsc;

use super::*;
use crate::hyperdht_messages::{FIREWALL_CONSISTENT, FIREWALL_UNKNOWN};
use crate::socket_pool::SocketPool;

fn peer(host: &str, port: u16) -> Ipv4Peer {
    Ipv4Peer {
        host: host.into(),
        port,
    }
}

/// A `Connected` event for `addr` plus a clone of its socket for `ptr_eq` checks.
async fn connected(addr: SocketAddr) -> (HolepunchEvent, UdxSocket) {
    let rt = libudx::UdxRuntime::new().expect("runtime");
    let socket = rt.create_socket().await.expect("socket");
    let keep = socket.clone();
    (HolepunchEvent::Connected { addr, socket }, keep)
}

#[tokio::test]
async fn connected_now_returns_the_address_and_the_receiving_socket() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let addr: SocketAddr = "203.0.113.7:7000".parse().unwrap();
    let (ev, sock) = connected(addr).await;
    tx.send(ev).unwrap();
    let (a, s) = connected_now(&mut rx).expect("connected");
    assert_eq!(a, addr);
    assert!(s.ptr_eq(&sock));
}

#[test]
fn connected_now_skips_empty_and_aborted() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    assert!(connected_now(&mut rx).is_none());
    tx.send(HolepunchEvent::Aborted).unwrap();
    assert!(connected_now(&mut rx).is_none());
}

#[test]
fn probe_carries_the_server_address() {
    let server = peer("203.0.113.7", 7000);
    let p = build_probe(FIREWALL_CONSISTENT, 0, vec![], &server, [1; 32], None);
    assert_eq!(p.remote_address, Some(server));
    assert!(!p.punching);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn probe_follows_a_low_ttl_packet_from_the_puzzle_socket() {
    let rt = libudx::UdxRuntime::new().expect("runtime");
    let cfg = crate::rpc::DhtConfig {
        bootstrap: vec![],
        host: "127.0.0.1".to_string(),
        ephemeral: Some(true),
        firewalled: true,
        ..crate::rpc::DhtConfig::default()
    };
    let (_task, dht) = crate::rpc::spawn(&rt, cfg).await.expect("spawn dht");

    let pool = SocketPool::new("127.0.0.1".to_string());
    let (tx, _rx) = mpsc::unbounded_channel();
    let puncher = Holepuncher::new(&pool, &rt, true, true, FIREWALL_UNKNOWN, tx)
        .await
        .expect("puncher");
    let sr = puncher.primary_socket().expect("primary socket");
    let via = sr.socket.clone();
    let puzzle_port = via.local_addr().await.expect("puzzle addr").port();

    // Bare socket stands in for the relay: a pool socket would drop requests.
    let relay_sock = rt.create_socket().await.expect("socket");
    relay_sock
        .bind("127.0.0.1:0".parse().unwrap())
        .await
        .expect("bind");
    let relay_addr = relay_sock.local_addr().await.expect("relay addr");
    let mut relay_rx = relay_sock.recv_start().expect("recv_start");
    let server = peer("127.0.0.1", relay_addr.port());

    let _ = tokio::time::timeout(
        Duration::from_millis(300),
        send_probe(
            &dht,
            &puncher,
            &server,
            &[0; 32],
            vec![1, 2, 3],
            &server,
            Some(via),
        ),
    )
    .await;

    let first = tokio::time::timeout(Duration::from_secs(1), relay_rx.recv())
        .await
        .expect("first datagram")
        .expect("lane open");
    let second = tokio::time::timeout(Duration::from_secs(1), relay_rx.recv())
        .await
        .expect("second datagram (the request)")
        .expect("lane open");
    assert_eq!(first.data, [0]);
    assert!(second.data.len() > 1);
    assert_eq!(first.addr.port(), puzzle_port);
    assert_eq!(second.addr.port(), puzzle_port);
    let _ = dht.destroy().await;
}

fn response() -> ResponseData {
    ResponseData {
        from: peer("127.0.0.1", 4001),
        id: None,
        token: None,
        closer_nodes: vec![],
        error: 0,
        value: None,
        rtt: Duration::ZERO,
        to: None,
    }
}

#[tokio::test(start_paused = true)]
async fn grace_for_unknown_pauses_one_second() {
    let t0 = tokio::time::Instant::now();
    grace_for_unknown(FIREWALL_UNKNOWN).await;
    assert_eq!(t0.elapsed(), Duration::from_secs(1));

    let t1 = tokio::time::Instant::now();
    grace_for_unknown(FIREWALL_CONSISTENT).await;
    assert_eq!(t1.elapsed(), Duration::ZERO);
}

#[tokio::test]
async fn connected_ends_or_connected_without_waiting() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let addr: SocketAddr = "203.0.113.7:7000".parse().unwrap();
    tx.send(connected(addr).await.0).unwrap();
    let raced = tokio::time::timeout(
        Duration::from_millis(200),
        or_connected(
            std::future::pending::<Result<ResponseData, DhtError>>(),
            &mut rx,
        ),
    )
    .await
    .expect("Connected must end the round without waiting for the reply");
    assert!(matches!(raced, Ok(Raced::Connected(a, _)) if a == addr));
}

#[tokio::test]
async fn connected_wins_when_future_is_ready() {
    let addr: SocketAddr = "203.0.113.7:7000".parse().unwrap();
    for i in 0..32 {
        let (tx, mut rx) = mpsc::unbounded_channel();
        tx.send(connected(addr).await.0).unwrap();
        let raced = or_connected(std::future::ready(Ok::<_, DhtError>(response())), &mut rx).await;
        assert!(
            matches!(raced, Ok(Raced::Connected(a, _)) if a == addr),
            "iteration {i}: Connected must beat a ready reply"
        );
    }
}

#[tokio::test]
async fn aborted_during_or_connected_is_an_error() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    tx.send(HolepunchEvent::Aborted).unwrap();
    let raced = tokio::time::timeout(
        Duration::from_millis(200),
        or_connected(
            std::future::pending::<Result<ResponseData, DhtError>>(),
            &mut rx,
        ),
    )
    .await
    .expect("Aborted must end the round");
    assert!(matches!(raced, Err(HyperDhtError::HolepunchAborted)));
}

#[tokio::test]
async fn connected_beats_pending_analyze_like_future() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let addr: SocketAddr = "203.0.113.7:7000".parse().unwrap();
    tx.send(connected(addr).await.0).unwrap();
    let raced = tokio::time::timeout(
        Duration::from_millis(200),
        or_connected(std::future::pending::<bool>(), &mut rx),
    )
    .await
    .expect("Connected must not wait for a pending future");
    assert!(matches!(raced, Ok(Raced::Connected(a, _)) if a == addr));
}

fn probe_reply(from: &Ipv4Peer, to: Option<Ipv4Peer>) -> ResponseData {
    ResponseData {
        from: from.clone(),
        id: None,
        token: None,
        closer_nodes: vec![],
        error: 0,
        value: None,
        rtt: Duration::from_millis(1),
        to,
    }
}

/// The probe reply feeds NAT with OUR reflexive address (`to`), not the server's.
#[test]
fn probe_reply_feeds_nat_with_the_reflexive_address() {
    let client_seen = peer("198.51.100.5", 4000);
    let mut nat = crate::nat::Nat::new(true);
    add_probe_sample(
        &mut nat,
        &probe_reply(&peer("10.0.0.1", 1), Some(client_seen.clone())),
    );
    nat.add(&client_seen, &peer("10.0.0.2", 2));
    nat.add(&client_seen, &peer("10.0.0.3", 3));
    assert_eq!(nat.sampled, 3);
    assert_eq!(nat.firewall, FIREWALL_CONSISTENT);
}

#[test]
fn probe_reply_without_to_adds_no_sample() {
    let mut nat = crate::nat::Nat::new(true);
    add_probe_sample(&mut nat, &probe_reply(&peer("10.0.0.1", 1), None));
    assert_eq!(nat.sampled, 0);
}

#[test]
fn final_reply_aborted_is_an_error() {
    assert!(matches!(
        final_reply_error(crate::hyperdht_messages::ERROR_ABORTED),
        Err(HyperDhtError::HolepunchAborted)
    ));
    assert!(final_reply_error(ERROR_NONE).is_ok());
    assert!(final_reply_error(crate::hyperdht_messages::ERROR_TRY_LATER).is_ok());
}
