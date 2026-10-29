//! Tests for the recv-adapter of [`super::Holepuncher`].

use super::*;
use libudx::UdxRuntime;
use std::time::Duration;
use tokio::time::timeout;

#[tokio::test]
async fn recv_adapter_initiator_emits_connected_on_first_probe() {
    let runtime = UdxRuntime::new().expect("runtime");
    let pool = SocketPool::new("127.0.0.1".to_string());

    let (event_tx, mut event_rx) = mpsc::unbounded_channel();

    let puncher = Holepuncher::new(&pool, &runtime, true, true, FIREWALL_CONSISTENT, event_tx)
        .await
        .expect("puncher");

    let primary = puncher.primary_socket().expect("primary socket");
    let primary_sock = primary.socket.clone();
    let target_addr = primary.socket.local_addr().await.expect("local_addr");

    let probe_socket = pool
        .acquire(&runtime)
        .await
        .expect("acquire")
        .socket
        .clone();
    let probe_local = probe_socket.local_addr().await.expect("probe local_addr");
    probe_socket
        .send_to(&[0u8], target_addr)
        .expect("send probe");

    let evt = timeout(Duration::from_secs(2), event_rx.recv())
        .await
        .expect("Connected within 2s")
        .expect("event_rx open");
    match evt {
        HolepunchEvent::Connected { addr, socket } => {
            assert_eq!(addr, probe_local, "Connected.addr matches probe source");
            assert!(
                socket.ptr_eq(&primary_sock),
                "Connected names the socket that received the probe"
            );
        }
        _ => panic!("expected Connected, got something else"),
    }

    probe_socket
        .send_to(&[0u8], target_addr)
        .expect("send second probe");
    let evt2 = timeout(Duration::from_millis(500), event_rx.recv()).await;
    assert!(evt2.is_err(), "second probe should not emit a second event");
}

/// Birthday-socket recv-adapter coverage: a probe landing on a
/// birthday-attack socket (opened by `open_birthday_sockets` for
/// RANDOM-class NAT punching) must fire `Connected`, not only a probe on
/// the primary socket. Registers a second socket exactly as
/// `open_birthday_sockets` does and confirms a probe delivered to *that*
/// socket (not the primary) still resolves the initiator's Connected
/// event.
#[tokio::test]
async fn recv_adapter_covers_birthday_sockets() {
    let runtime = UdxRuntime::new().expect("runtime");
    let pool = SocketPool::new("127.0.0.1".to_string());
    let (event_tx, mut event_rx) = mpsc::unbounded_channel();

    let mut puncher = Holepuncher::new(&pool, &runtime, true, true, FIREWALL_RANDOM, event_tx)
        .await
        .expect("puncher");

    // Register a second ("birthday") socket exactly as
    // `open_birthday_sockets` would, without running the full punch loop.
    let mut extra = pool.acquire(&runtime).await.expect("acquire extra socket");
    let extra_addr = extra.socket.local_addr().await.expect("extra local_addr");
    let extra_sock = extra.socket.clone();
    if let Some(handle) = spawn_recv_adapter_for_socket(
        &mut extra,
        puncher.is_initiator,
        puncher.event_tx.clone(),
        Arc::clone(&puncher.connected_flag),
    ) {
        puncher.recv_tasks.push(handle);
    }
    puncher.sockets.push(extra);
    assert_eq!(puncher.sockets.len(), 2, "primary + one birthday socket");

    // Send a probe to the SECOND (birthday) socket, not the primary one.
    let probe_socket = pool
        .acquire(&runtime)
        .await
        .expect("acquire probe socket")
        .socket
        .clone();
    let probe_local = probe_socket.local_addr().await.expect("probe local_addr");
    probe_socket
        .send_to(&[0u8], extra_addr)
        .expect("send probe to birthday socket");

    let evt = timeout(Duration::from_secs(2), event_rx.recv())
        .await
        .expect("Connected within 2s")
        .expect("event_rx open");
    match evt {
        HolepunchEvent::Connected { addr, socket } => {
            assert_eq!(
                addr, probe_local,
                "Connected fires from a probe landing on the birthday socket"
            );
            assert!(
                socket.ptr_eq(&extra_sock),
                "Connected names the birthday socket, not the primary one"
            );
        }
        _ => panic!("expected Connected, got something else"),
    }
}

#[tokio::test]
async fn recv_adapter_passive_reflects_probe() {
    let runtime = UdxRuntime::new().expect("runtime");
    let pool = SocketPool::new("127.0.0.1".to_string());

    let (event_tx, _event_rx) = mpsc::unbounded_channel();

    let puncher = Holepuncher::new(&pool, &runtime, true, false, FIREWALL_CONSISTENT, event_tx)
        .await
        .expect("puncher");

    let primary = puncher.primary_socket().expect("primary socket");
    let puncher_addr = primary.socket.local_addr().await.expect("local_addr");

    let mut probe_socket_ref = pool.acquire(&runtime).await.expect("acquire");
    let probe_socket = probe_socket_ref.socket.clone();
    let mut probe_hp_rx = probe_socket_ref.take_holepunch_rx().expect("rx");

    probe_socket
        .send_to(&[0u8], puncher_addr)
        .expect("send probe");

    let reflection = timeout(Duration::from_secs(2), probe_hp_rx.recv())
        .await
        .expect("reflection within 2s")
        .expect("rx open");

    assert_eq!(
        reflection.addr, puncher_addr,
        "reflection comes from puncher's address"
    );
}

#[tokio::test]
async fn on_holepunch_message_names_the_socket_and_skips_unknown_index() {
    let runtime = UdxRuntime::new().expect("runtime");
    let pool = SocketPool::new("127.0.0.1".to_string());
    let (event_tx, mut event_rx) = mpsc::unbounded_channel();

    let mut p = Holepuncher::new(&pool, &runtime, true, true, FIREWALL_CONSISTENT, event_tx)
        .await
        .expect("puncher");
    let primary = p.primary_socket().expect("primary socket").socket.clone();
    let addr: SocketAddr = "203.0.113.7:7000".parse().unwrap();

    p.on_holepunch_message(addr, 99);
    assert!(event_rx.try_recv().is_err(), "unknown index emits nothing");
    assert!(!p.connected, "unknown index leaves the state alone");

    p.on_holepunch_message(addr, 0);
    match event_rx.try_recv() {
        Ok(HolepunchEvent::Connected { socket, .. }) => assert!(socket.ptr_eq(&primary)),
        _ => panic!("expected Connected"),
    }
}
