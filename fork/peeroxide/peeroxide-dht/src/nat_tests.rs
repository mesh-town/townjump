//! Tests for [`crate::nat`].

use super::*;
use std::time::Duration;
use tokio::sync::mpsc;

fn peer(host: &str, port: u16) -> Ipv4Peer {
    Ipv4Peer {
        host: host.into(),
        port,
    }
}

#[test]
fn new_firewalled_starts_unknown() {
    let nat = Nat::new(true);
    assert_eq!(nat.firewall, FIREWALL_UNKNOWN);
    assert!(nat.addresses.is_none());
    assert_eq!(nat.sampled, 0);
}

#[test]
fn new_not_firewalled_starts_open() {
    let nat = Nat::new(false);
    assert_eq!(nat.firewall, FIREWALL_OPEN);
}

#[test]
fn add_sample_sorting() {
    let mut samples = Vec::new();
    add_sample(&mut samples, "1.2.3.4", 1000);
    add_sample(&mut samples, "5.6.7.8", 2000);
    add_sample(&mut samples, "1.2.3.4", 1000);
    assert_eq!(samples[0].host, "1.2.3.4");
    assert_eq!(samples[0].hits, 2);
    assert_eq!(samples[1].host, "5.6.7.8");
    assert_eq!(samples[1].hits, 1);
}

#[test]
fn add_sample_triple_hit_stays_sorted() {
    let mut samples = Vec::new();
    add_sample(&mut samples, "a", 1);
    add_sample(&mut samples, "b", 2);
    add_sample(&mut samples, "c", 3);
    add_sample(&mut samples, "b", 2);
    add_sample(&mut samples, "b", 2);
    assert_eq!(samples[0].host, "b");
    assert_eq!(samples[0].hits, 3);
}

#[test]
fn consistent_nat_three_same_port() {
    let mut nat = Nat::new(true);
    nat.add(&peer("1.2.3.4", 5000), &peer("10.0.0.1", 1));
    nat.add(&peer("1.2.3.4", 5000), &peer("10.0.0.2", 2));
    nat.add(&peer("1.2.3.4", 5000), &peer("10.0.0.3", 3));
    assert_eq!(nat.firewall, FIREWALL_CONSISTENT);
    assert!(nat.addresses.is_some());
    let addrs = nat.addresses.as_ref().unwrap();
    assert_eq!(addrs[0].host, "1.2.3.4");
    assert_eq!(addrs[0].port, 5000);
}

#[test]
fn random_nat_all_different_ports() {
    let mut nat = Nat::new(true);
    nat.add(&peer("1.2.3.4", 5000), &peer("10.0.0.1", 1));
    nat.add(&peer("1.2.3.4", 5001), &peer("10.0.0.2", 2));
    nat.add(&peer("1.2.3.4", 5002), &peer("10.0.0.3", 3));
    assert_eq!(nat.firewall, FIREWALL_RANDOM);
    let addrs = nat.addresses.as_ref().unwrap();
    assert_eq!(addrs.len(), 1);
    assert_eq!(addrs[0].host, "1.2.3.4");
}

#[test]
fn duplicate_from_ignored() {
    let mut nat = Nat::new(true);
    let from = peer("10.0.0.1", 1);
    nat.add(&peer("1.2.3.4", 5000), &from);
    nat.add(&peer("1.2.3.4", 5000), &from);
    assert_eq!(nat.sampled, 1);
}

#[test]
fn freeze_prevents_update() {
    let mut nat = Nat::new(true);
    nat.freeze();
    nat.add(&peer("1.2.3.4", 5000), &peer("10.0.0.1", 1));
    nat.add(&peer("1.2.3.4", 5000), &peer("10.0.0.2", 2));
    nat.add(&peer("1.2.3.4", 5000), &peer("10.0.0.3", 3));
    assert_eq!(nat.firewall, FIREWALL_UNKNOWN);
    assert!(nat.addresses.is_none());

    nat.unfreeze();
    assert_eq!(nat.firewall, FIREWALL_CONSISTENT);
    assert!(nat.addresses.is_some());
}

#[test]
fn not_firewalled_always_open() {
    let mut nat = Nat::new(false);
    nat.add(&peer("1.2.3.4", 5000), &peer("10.0.0.1", 1));
    assert_eq!(nat.firewall, FIREWALL_OPEN);
}

#[test]
fn mark_visited_dedup() {
    let mut nat = Nat::new(true);
    assert!(nat.mark_visited("1.2.3.4", 1000));
    assert!(!nat.mark_visited("1.2.3.4", 1000));
}

#[test]
fn two_hits_multi_host_consistent() {
    let mut nat = Nat::new(true);
    nat.add(&peer("1.2.3.4", 5000), &peer("10.0.0.1", 1));
    nat.add(&peer("1.2.3.4", 5000), &peer("10.0.0.2", 2));
    nat.add(&peer("5.6.7.8", 5000), &peer("10.0.0.3", 3));
    // max=2, hosts>1, full[1].hits=1 → not enough evidence yet
    assert_eq!(nat.firewall, FIREWALL_UNKNOWN);

    nat.add(&peer("5.6.7.8", 5000), &peer("10.0.0.4", 4));
    // full[1].hits=2 with hosts>1 → consistent
    assert_eq!(nat.firewall, FIREWALL_CONSISTENT);
}

#[test]
fn two_hits_single_host_over_three_random() {
    let mut nat = Nat::new(true);
    nat.add(&peer("1.2.3.4", 5000), &peer("10.0.0.1", 1));
    nat.add(&peer("1.2.3.4", 5000), &peer("10.0.0.2", 2));
    nat.add(&peer("1.2.3.4", 5001), &peer("10.0.0.3", 3));
    // max=2, 1 host, sampled=3 → not enough evidence
    assert_eq!(nat.firewall, FIREWALL_UNKNOWN);

    nat.add(&peer("1.2.3.4", 5002), &peer("10.0.0.4", 4));
    // max=2, 1 host, sampled>3 → random
    assert_eq!(nat.firewall, FIREWALL_RANDOM);
}

#[test]
fn over_four_samples_no_decision_random() {
    let mut nat = Nat::new(true);
    nat.add(&peer("1.2.3.4", 5000), &peer("10.0.0.1", 1));
    nat.add(&peer("5.6.7.8", 6000), &peer("10.0.0.2", 2));
    nat.add(&peer("1.2.3.4", 5000), &peer("10.0.0.3", 3));
    assert_eq!(nat.firewall, FIREWALL_UNKNOWN);

    nat.add(&peer("9.9.9.9", 7000), &peer("10.0.0.4", 4));
    assert_eq!(nat.firewall, FIREWALL_UNKNOWN);

    nat.add(&peer("8.8.8.8", 8000), &peer("10.0.0.5", 5));
    // sampled>4, no strong signal → random
    assert_eq!(nat.firewall, FIREWALL_RANDOM);
}

#[test]
fn update_resets_open_if_firewalled() {
    let mut nat = Nat::new(false);
    assert_eq!(nat.firewall, FIREWALL_OPEN);

    nat.firewalled = true;
    nat.update();
    assert_eq!(nat.firewall, FIREWALL_UNKNOWN);
}

#[test]
fn consistent_addresses_include_high_hit_entries() {
    let mut nat = Nat::new(true);
    nat.add(&peer("1.2.3.4", 5000), &peer("10.0.0.1", 1));
    nat.add(&peer("1.2.3.4", 5000), &peer("10.0.0.2", 2));
    nat.add(&peer("1.2.3.4", 5000), &peer("10.0.0.3", 3));
    assert_eq!(nat.firewall, FIREWALL_CONSISTENT);

    let addrs = nat.addresses.as_ref().unwrap();
    assert_eq!(addrs.len(), 1);
    assert_eq!(addrs[0].port, 5000);

    nat.add(&peer("1.2.3.4", 6000), &peer("10.0.0.4", 4));
    let addrs = nat.addresses.as_ref().unwrap();
    assert!(addrs.len() >= 2);
}

#[tokio::test(flavor = "current_thread")]
async fn analyzing_resolves_on_consistent() {
    let mut nat = Nat::new(true);
    let analyzing = nat.analyzing();

    nat.add(&peer("1.2.3.4", 5000), &peer("100.0.0.1", 1));
    nat.add(&peer("1.2.3.4", 5000), &peer("100.0.0.2", 1));
    nat.add(&peer("1.2.3.4", 5000), &peer("100.0.0.3", 1));

    assert_eq!(nat.firewall, FIREWALL_CONSISTENT);

    tokio::time::timeout(std::time::Duration::from_millis(100), analyzing)
        .await
        .expect("analyzing did not resolve after CONSISTENT classification");
}

#[tokio::test(flavor = "current_thread")]
async fn analyzing_resolves_on_min_samples_exhaustion() {
    let mut nat = Nat::new(true);
    let analyzing = nat.analyzing();

    nat.add(&peer("1.2.3.4", 1), &peer("100.0.0.1", 1));
    nat.add(&peer("1.2.3.4", 2), &peer("100.0.0.2", 1));
    nat.add(&peer("1.2.3.4", 3), &peer("100.0.0.3", 1));
    nat.add(&peer("1.2.3.4", 4), &peer("100.0.0.4", 1));

    assert!(nat.sampled >= NAT_MIN_SAMPLES);

    tokio::time::timeout(std::time::Duration::from_millis(100), analyzing)
        .await
        .expect("analyzing did not resolve after MIN_SAMPLES exhaustion");
}

#[tokio::test(flavor = "current_thread")]
async fn analyzing_resolves_on_destroy() {
    let mut nat = Nat::new(true);
    let analyzing = nat.analyzing();

    nat.destroy();

    tokio::time::timeout(std::time::Duration::from_millis(100), analyzing)
        .await
        .expect("analyzing did not resolve after destroy()");
}

#[tokio::test(flavor = "current_thread")]
async fn settled_state_after_three_consistent_samples() {
    let mut nat = Nat::new(true);
    nat.add(&peer("1.2.3.4", 5000), &peer("10.0.0.1", 1));
    nat.add(&peer("1.2.3.4", 5000), &peer("10.0.0.2", 2));
    nat.add(&peer("1.2.3.4", 5000), &peer("10.0.0.3", 3));
    assert_eq!(nat.firewall, FIREWALL_CONSISTENT);
    assert!(
        nat.firewall != FIREWALL_UNKNOWN,
        "settled NAT must not be UNKNOWN — auto_sample short-circuits in this state"
    );
}

/// Drives `Nat::auto_sample` end-to-end against real loopback DHT nodes
/// (no internet required). A primary node's routing table is seeded
/// with `NAT_MIN_SAMPLES` local peer nodes via
/// `DhtHandle::insert_node_for_test` (a test-only helper), each backed
/// by a genuine `rpc::spawn`-ed DHT actor bound to `127.0.0.1` that can
/// answer the internal `ping_via_socket` FIND_NODE probe. This exercises
/// the concurrent-ping, reflexive-`to`/`from` sample-collection, and
/// puncher-socket reply-forwarding paths without any mock trait — the
/// "mock" is simply a same-host, no-bootstrap DHT peer.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auto_sample_collects_target_samples() {
    use crate::routing_table::Node;
    use crate::socket_pool::SocketPool;

    let runtime = libudx::UdxRuntime::new().expect("runtime");

    // Primary node under test: firewalled, no bootstrap, ephemeral.
    let primary_cfg = crate::rpc::DhtConfig {
        bootstrap: vec![],
        host: "127.0.0.1".to_string(),
        ephemeral: Some(true),
        firewalled: true,
        ..crate::rpc::DhtConfig::default()
    };
    let (_primary_task, primary) = crate::rpc::spawn(&runtime, primary_cfg)
        .await
        .expect("spawn primary");

    // Ping targets: independent loopback DHT nodes that can answer a
    // FIND_NODE probe sent via a puncher socket.
    let target_count = NAT_MIN_SAMPLES as usize;
    let mut target_tasks = Vec::new();
    let mut target_handles = Vec::new();
    for _ in 0..target_count {
        let cfg = crate::rpc::DhtConfig {
            bootstrap: vec![],
            host: "127.0.0.1".to_string(),
            ephemeral: Some(true),
            firewalled: false,
            ..crate::rpc::DhtConfig::default()
        };
        let (task, handle) = crate::rpc::spawn(&runtime, cfg)
            .await
            .expect("spawn target");
        target_tasks.push(task);
        target_handles.push(handle);
    }

    // Seed the primary's routing table directly (bypassing discovery)
    // so `recent_nodes()` returns our loopback targets deterministically.
    for handle in &target_handles {
        let port = handle.local_port().await.expect("target local_port");
        let node = Node {
            id: rand::random(),
            host: "127.0.0.1".to_string(),
            port,
            token: None,
            added_tick: 0,
            seen_tick: 0,
            pinged_tick: 0,
            down_hints: 0,
        };
        primary.insert_node_for_test(node);
    }

    // Acquire a puncher socket exactly as production code does, wiring
    // its DHT-reply lane through to `auto_sample`.
    let pool = SocketPool::new("127.0.0.1".to_string());
    let mut socket_ref = pool
        .acquire(&runtime)
        .await
        .expect("acquire puncher socket");
    let socket = socket_ref.socket.clone();
    let dht_reply_rx = socket_ref
        .take_dht_reply_rx()
        .expect("dht_reply_rx available");

    crate::nat_sampler::spawn_dht_reply_forwarder(primary.clone(), dht_reply_rx);
    let mut nat = Nat::new(true);
    let added = tokio::time::timeout(
        Duration::from_secs(10),
        nat.auto_sample(&primary, socket, target_count),
    )
    .await
    .expect("auto_sample timed out");

    // All targets share one reflexive loopback address, so the NAT
    // classifier can settle on FIREWALL_CONSISTENT after 3 matching
    // samples and `auto_sample` breaks out early (mirrors Node's
    // early-exit once `self.firewall != FIREWALL_UNKNOWN`) — it need
    // not visit every candidate to prove samples are being collected.
    assert!(
        added >= 3,
        "expected auto_sample to collect at least 3 samples from reachable loopback targets, got {added}"
    );
    assert!(
        nat.sampled >= 3,
        "Nat::add should have been driven by successful ping_via_socket replies"
    );
    assert_eq!(
        nat.firewall, FIREWALL_CONSISTENT,
        "identical reflexive address across all targets should settle as CONSISTENT"
    );

    let _ = primary.destroy().await;
    for handle in &target_handles {
        let _ = handle.destroy().await;
    }
}

/// Loopback DHT node without bootstrap, as in `auto_sample_collects_target_samples`.
async fn loopback_dht(
    runtime: &libudx::UdxRuntime,
    firewalled: bool,
) -> (
    tokio::task::JoinHandle<Result<(), crate::rpc::DhtError>>,
    DhtHandle,
) {
    let cfg = crate::rpc::DhtConfig {
        bootstrap: vec![],
        host: "127.0.0.1".to_string(),
        ephemeral: Some(true),
        firewalled,
        ..crate::rpc::DhtConfig::default()
    };
    let (task, handle) = crate::rpc::spawn(runtime, cfg).await.expect("spawn dht");
    (task, handle)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_via_reply_arrives_after_auto_sample() {
    use crate::socket_pool::SocketPool;

    let runtime = libudx::UdxRuntime::new().expect("runtime");
    let (_task, primary) = loopback_dht(&runtime, true).await;
    let (_t, target) = loopback_dht(&runtime, false).await;
    let target_port = target.local_port().await.expect("target port");
    let pool = SocketPool::new("127.0.0.1".to_string());
    let (etx, _erx) = mpsc::unbounded_channel();
    let mut puncher = crate::holepuncher::Holepuncher::new(&pool, &runtime, false, true, 0, etx)
        .await
        .expect("puncher");
    let socket = puncher.primary_socket().expect("primary").socket.clone();
    let added = puncher.auto_sample(&primary).await;
    assert_eq!(added, 0, "NAT OPEN exits auto_sample early");

    let params = crate::rpc::UserRequestParams {
        command: crate::messages::Command::Ping as u64,
        token: None,
        target: None,
        value: None,
        timeout_ms: None,
        retries: None,
    };
    let reply = tokio::time::timeout(
        Duration::from_secs(2),
        primary.request_via(params, "127.0.0.1", target_port, socket),
    )
    .await;
    assert!(
        matches!(reply, Ok(Ok(_))),
        "reply to a request from the puzzle socket must arrive after the early exit: {reply:?}"
    );
    let _ = primary.destroy().await;
    let _ = target.destroy().await;
}
