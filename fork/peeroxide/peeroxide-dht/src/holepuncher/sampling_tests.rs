//! Tests for [`super`] (background NAT sampling of the `Holepuncher`).

use std::time::Duration;

use tokio::sync::mpsc;

use crate::holepuncher::Holepuncher;
use crate::hyperdht_messages::FIREWALL_UNKNOWN;
use crate::rpc::{DhtConfig, DhtHandle};
use crate::socket_pool::SocketPool;

async fn loopback_dht(
    runtime: &libudx::UdxRuntime,
    firewalled: bool,
) -> (
    tokio::task::JoinHandle<Result<(), crate::rpc::DhtError>>,
    DhtHandle,
) {
    let cfg = DhtConfig {
        bootstrap: vec![],
        host: "127.0.0.1".to_string(),
        ephemeral: Some(true),
        firewalled,
        ..DhtConfig::default()
    };
    crate::rpc::spawn(runtime, cfg).await.expect("spawn dht")
}

fn node_at(port: u16) -> crate::routing_table::Node {
    crate::routing_table::Node {
        id: rand::random(),
        host: "127.0.0.1".to_string(),
        port,
        token: None,
        added_tick: 0,
        seen_tick: 0,
        pinged_tick: 0,
        down_hints: 0,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reply_via_puzzle_socket_arrives_before_sampling_ends() {
    let runtime = libudx::UdxRuntime::new().expect("runtime");
    let (_pt, primary) = loopback_dht(&runtime, true).await;
    let (_tt, target) = loopback_dht(&runtime, false).await;
    let target_port = target.local_port().await.expect("target port");

    // Black hole: never answers, so the sampler stays busy for its whole 2 s timeout.
    let hole = tokio::net::UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("hole");
    primary.insert_node_for_test(node_at(hole.local_addr().unwrap().port()));

    let pool = SocketPool::new("127.0.0.1".to_string());
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut puncher = Holepuncher::new(&pool, &runtime, true, true, FIREWALL_UNKNOWN, tx)
        .await
        .expect("puncher");
    puncher.start_background_sampling(&primary);

    let socket = puncher.primary_socket().expect("primary").socket.clone();
    let params = crate::rpc::UserRequestParams {
        command: crate::messages::Command::Ping as u64,
        token: None,
        target: None,
        value: None,
        timeout_ms: None,
        retries: None,
    };
    let reply = tokio::time::timeout(
        Duration::from_millis(1500),
        primary.request_via(params, "127.0.0.1", target_port, socket),
    )
    .await;
    assert!(
        matches!(reply, Ok(Ok(_))),
        "reply on the puzzle socket must arrive while sampling is still running: {reply:?}"
    );
    assert!(
        matches!(
            puncher.samples.as_mut().expect("sampler").try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ),
        "sampler still waits on the black hole (channel alive, no pongs)"
    );
    drop(hole);
    let _ = primary.destroy().await;
    let _ = target.destroy().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn analyze_waits_for_samples_then_settles() {
    let runtime = libudx::UdxRuntime::new().expect("runtime");
    let (_pt, primary) = loopback_dht(&runtime, true).await;
    let mut targets = Vec::new();
    for _ in 0..4 {
        let (task, handle) = loopback_dht(&runtime, false).await;
        let port = handle.local_port().await.expect("target port");
        primary.insert_node_for_test(node_at(port));
        targets.push((task, handle));
    }

    let pool = SocketPool::new("127.0.0.1".to_string());
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut puncher = Holepuncher::new(&pool, &runtime, true, true, FIREWALL_UNKNOWN, tx)
        .await
        .expect("puncher");
    puncher.start_background_sampling(&primary);

    tokio::time::timeout(Duration::from_secs(5), puncher.analyze(false))
        .await
        .expect("analyze must return once sampling settles");
    assert!(puncher.nat.sampled >= 3, "sampled {}", puncher.nat.sampled);
    assert!(puncher.nat.is_settled());

    let _ = primary.destroy().await;
    for (_, h) in &targets {
        let _ = h.destroy().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn analyze_returns_when_no_nodes() {
    let runtime = libudx::UdxRuntime::new().expect("runtime");
    let (_pt, primary) = loopback_dht(&runtime, true).await;

    let pool = SocketPool::new("127.0.0.1".to_string());
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut puncher = Holepuncher::new(&pool, &runtime, true, true, FIREWALL_UNKNOWN, tx)
        .await
        .expect("puncher");
    puncher.start_background_sampling(&primary);

    // The result is irrelevant; the call must not hang on an empty table.
    tokio::time::timeout(Duration::from_millis(500), puncher.analyze(false))
        .await
        .expect("analyze must return when there is nothing to sample");
    let _ = primary.destroy().await;
}

/// Starts a sampler against a single black-hole node (never answers, 2 s ping timeout).
async fn puncher_on_black_hole(
    runtime: &libudx::UdxRuntime,
    primary: &DhtHandle,
) -> (Holepuncher, tokio::net::UdpSocket) {
    let hole = tokio::net::UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("hole");
    primary.insert_node_for_test(node_at(hole.local_addr().unwrap().port()));
    let pool = SocketPool::new("127.0.0.1".to_string());
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut puncher = Holepuncher::new(&pool, runtime, true, true, FIREWALL_UNKNOWN, tx)
        .await
        .expect("puncher");
    puncher.start_background_sampling(primary);
    (puncher, hole)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn absorb_samples_finishes_when_sampler_is_done() {
    let runtime = libudx::UdxRuntime::new().expect("runtime");
    let (_pt, primary) = loopback_dht(&runtime, true).await;
    // Empty routing table: the sampler's channel closes at once.
    let pool = SocketPool::new("127.0.0.1".to_string());
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut puncher = Holepuncher::new(&pool, &runtime, true, true, FIREWALL_UNKNOWN, tx)
        .await
        .expect("puncher");
    puncher.start_background_sampling(&primary);
    assert!(puncher.samples.is_some());

    let deadline = tokio::time::Instant::now() + Duration::from_millis(1500);
    while puncher.samples.is_some() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
        puncher.absorb_samples();
    }
    assert!(puncher.samples.is_none(), "closed channel ends sampling");
    assert!(puncher.nat.sampling_settled());
    let _ = primary.destroy().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn destroy_during_sampling_releases_analyze() {
    let runtime = libudx::UdxRuntime::new().expect("runtime");
    let (_pt, primary) = loopback_dht(&runtime, true).await;
    let (mut puncher, hole) = puncher_on_black_hole(&runtime, &primary).await;
    puncher.destroy();
    tokio::time::timeout(Duration::from_millis(500), puncher.analyze(false))
        .await
        .expect("analyze must return at once after destroy");
    drop(hole);
    let _ = primary.destroy().await;
}

/// A reply received on the puzzle socket carries our reflexive address in `to`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_via_reply_carries_the_reflexive_address() {
    let runtime = libudx::UdxRuntime::new().expect("runtime");
    let (_pt, primary) = loopback_dht(&runtime, true).await;
    let (_tt, target) = loopback_dht(&runtime, false).await;
    let target_port = target.local_port().await.expect("target port");
    let pool = SocketPool::new("127.0.0.1".to_string());
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut puncher = Holepuncher::new(&pool, &runtime, true, true, FIREWALL_UNKNOWN, tx)
        .await
        .expect("puncher");
    // Routes replies of the puzzle socket back to the DHT (as in the connect flow).
    puncher.start_background_sampling(&primary);
    let socket = puncher.primary_socket().expect("primary").socket.clone();
    let our_port = socket.local_addr().await.expect("addr").port();
    let params = crate::rpc::UserRequestParams {
        command: crate::messages::Command::Ping as u64,
        ..Default::default()
    };
    let reply = primary
        .request_via(params, "127.0.0.1", target_port, socket)
        .await
        .expect("reply");
    let to = reply.to.expect("`to` must reach ResponseData");
    assert_eq!((to.host.as_str(), to.port), ("127.0.0.1", our_port));
    let _ = primary.destroy().await;
    let _ = target.destroy().await;
}

/// The probe reply of the relay lands before the first `absorb_samples`: with the
/// reflexive address it is a normal sample, so three nodes still give CONSISTENT.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn probe_reply_before_absorb_keeps_the_nat_consistent() {
    let runtime = libudx::UdxRuntime::new().expect("runtime");
    let (_pt, primary) = loopback_dht(&runtime, true).await;
    let mut targets = Vec::new();
    for _ in 0..3 {
        let (task, handle) = loopback_dht(&runtime, false).await;
        let port = handle.local_port().await.expect("target port");
        primary.insert_node_for_test(node_at(port));
        targets.push((task, handle, port));
    }
    let pool = SocketPool::new("127.0.0.1".to_string());
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut puncher = Holepuncher::new(&pool, &runtime, true, true, FIREWALL_UNKNOWN, tx)
        .await
        .expect("puncher");
    puncher.start_background_sampling(&primary);
    // The pongs sit in the channel; nothing has absorbed them yet.
    tokio::time::sleep(Duration::from_millis(400)).await;

    let our = puncher
        .primary_socket()
        .expect("primary")
        .socket
        .local_addr()
        .await
        .expect("addr");
    let peer_of = |host: &str, port: u16| crate::messages::Ipv4Peer {
        host: host.to_string(),
        port,
    };
    let relay = peer_of("127.0.0.1", targets[0].2);
    let resp = crate::rpc::ResponseData {
        from: relay.clone(),
        id: None,
        token: None,
        closer_nodes: vec![],
        error: 0,
        value: None,
        rtt: Duration::from_millis(1),
        to: Some(peer_of("127.0.0.1", our.port())),
    };
    crate::hyperdht::connect_fast_open::add_probe_sample(&mut puncher.nat, &resp);

    tokio::time::timeout(Duration::from_secs(5), puncher.analyze(false))
        .await
        .expect("analyze returns");
    assert_eq!(puncher.nat.sampled, 3);
    assert_eq!(
        puncher.nat.firewall,
        crate::hyperdht_messages::FIREWALL_CONSISTENT
    );

    let _ = primary.destroy().await;
    for (_, h, _) in &targets {
        let _ = h.destroy().await;
    }
}
