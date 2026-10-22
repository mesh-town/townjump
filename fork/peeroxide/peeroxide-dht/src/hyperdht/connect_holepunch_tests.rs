//! Tests for [`super`] (client holepunch rounds).

use std::sync::Arc;
use std::time::Duration;

use libudx::{Datagram, UdxRuntime, UdxSocket};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::task::JoinHandle;

use crate::hyperdht::config::HyperDhtConfig;
use crate::hyperdht::connection::ConnectResult;
use crate::hyperdht::handle::HyperDhtHandle;
use crate::hyperdht::types::HyperDhtError;
use crate::hyperdht_messages::{FIREWALL_CONSISTENT, HolepunchInfo, NoisePayload};
use crate::messages::Ipv4Peer;
use crate::noise_wrap::NoiseWrapResult;
use crate::rpc::DhtConfig;

type Rounds = JoinHandle<Result<ConnectResult, HyperDhtError>>;

struct Harness {
    runtime: Arc<UdxRuntime>,
    handle: HyperDhtHandle,
    hole: tokio::net::UdpSocket,
    relay_sock: UdxSocket,
    relay_rx: UnboundedReceiver<Datagram>,
    relay: Ipv4Peer,
}

/// Node without bootstrap, a black hole in its table, and a bare socket standing in for the relay.
async fn harness() -> Harness {
    let runtime = Arc::new(UdxRuntime::new().expect("runtime"));
    let cfg = HyperDhtConfig {
        dht: DhtConfig {
            bootstrap: vec![],
            host: "127.0.0.1".to_string(),
            ephemeral: Some(true),
            firewalled: true,
            ..DhtConfig::default()
        },
        ..HyperDhtConfig::default()
    };
    let (_task, handle, _events) = crate::hyperdht::node::spawn(&runtime, cfg)
        .await
        .expect("spawn hyperdht");

    // Black hole in the routing table: the sampler's ping never gets an answer (2 s timeout).
    let hole = tokio::net::UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("hole");
    handle.dht.insert_node_for_test(crate::routing_table::Node {
        id: rand::random(),
        host: "127.0.0.1".to_string(),
        port: hole.local_addr().unwrap().port(),
        token: None,
        added_tick: 0,
        seen_tick: 0,
        pinged_tick: 0,
        down_hints: 0,
    });

    // Bare socket stands in for the relay: a pool socket would drop requests.
    let relay_sock = runtime.create_socket().await.expect("socket");
    relay_sock
        .bind("127.0.0.1:0".parse().unwrap())
        .await
        .expect("bind");
    let relay_addr = relay_sock.local_addr().await.expect("relay addr");
    let relay_rx = relay_sock.recv_start().expect("recv_start");
    let relay = Ipv4Peer {
        host: "127.0.0.1".into(),
        port: relay_addr.port(),
    };
    Harness {
        runtime,
        handle,
        hole,
        relay_sock,
        relay_rx,
        relay,
    }
}

fn nw_result() -> NoiseWrapResult {
    NoiseWrapResult {
        is_initiator: true,
        remote_public_key: [1; 32],
        holepunch_secret: [7; 32],
        handshake_hash: [0; 64],
        tx: [0; 32],
        rx: [0; 32],
    }
}

fn remote_payload() -> NoisePayload {
    NoisePayload {
        version: 1,
        error: 0,
        firewall: FIREWALL_CONSISTENT,
        holepunch: Some(HolepunchInfo {
            id: 1,
            relays: vec![],
        }),
        addresses4: vec![],
        addresses6: vec![],
        udx: None,
        secret_stream: None,
        relay_through: None,
        relay_addresses: None,
    }
}

/// Starts the rounds; open_session goes to the server's puzzle address: the same bare socket.
fn start_rounds(h: &Harness) -> Rounds {
    let handle = h.handle.clone();
    let relay = h.relay.clone();
    let runtime = Arc::clone(&h.runtime);
    let (nw, payload) = (nw_result(), remote_payload());
    tokio::spawn(async move {
        handle
            .run_holepunch_rounds(&nw, &payload, &relay, &[9; 32], &relay, &runtime, 1, None)
            .await
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn probe_is_sent_before_sampling_ends() {
    let mut h = harness().await;
    let rounds = start_rounds(&h);

    // One shared deadline for both datagrams; below the ~1 s the black-hole ping takes to fail.
    let deadline = tokio::time::Instant::now() + Duration::from_millis(800);
    let first = tokio::time::timeout_at(deadline, h.relay_rx.recv()).await;
    let second = tokio::time::timeout_at(deadline, h.relay_rx.recv()).await;
    rounds.abort();
    let _ = h.handle.dht.destroy().await;
    drop(h.hole);

    let first = first
        .expect("open_session packet within 800 ms")
        .expect("lane open");
    let second = second
        .expect("probe request within 800 ms, before sampling ends")
        .expect("lane open");
    assert_eq!(first.data.len(), 1);
    assert!(second.data.len() > 1);
}

/// Silent relay; after the probe request the server's 1-byte packet reaches the client socket.
async fn silent_relay_then_packet(h: &mut Harness) -> Rounds {
    let rounds = start_rounds(h);
    let first = h.relay_rx.recv().await.expect("open_session packet");
    assert_eq!(first.data.len(), 1);
    let second = h.relay_rx.recv().await.expect("probe request");
    assert!(second.data.len() > 1);
    h.relay_sock
        .send_to(&[0], second.addr)
        .expect("server packet");
    rounds
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connected_while_the_relay_is_silent_ends_the_rounds() {
    let mut h = harness().await;
    let rounds = silent_relay_then_packet(&mut h).await;

    // A naive implementation waits for the relay's reply: `Err` after ~4 s of retries.
    let result = tokio::time::timeout(Duration::from_secs(2), rounds)
        .await
        .expect("rounds end within 2 s of the packet")
        .expect("task");
    let _ = h.handle.dht.destroy().await;
    drop(h.hole);

    let result = result.expect("punched");
    assert_eq!(result.server_address.port, h.relay.port);
}

/// Slow (1.3 s, one real retry interval): pins the drop-cancels-the-request decision.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_cancelled_probe_request_is_not_retried() {
    let mut h = harness().await;
    let rounds = silent_relay_then_packet(&mut h).await;
    let result = tokio::time::timeout(Duration::from_secs(2), rounds)
        .await
        .expect("rounds end within 2 s of the packet")
        .expect("task");
    assert!(result.is_ok());

    tokio::time::sleep(Duration::from_millis(1300)).await;
    let mut retried = false;
    while let Ok(d) = h.relay_rx.try_recv() {
        retried |= d.data.len() > 1;
    }
    let _ = h.handle.dht.destroy().await;
    drop(h.hole);
    assert!(!retried, "request was repeated after Connected");
}
