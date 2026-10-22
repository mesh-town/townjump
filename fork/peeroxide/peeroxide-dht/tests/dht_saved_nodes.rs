// JS: dht-rpc/index.js (constructor `opts.nodes`, `toArray`)
//! Routing table seeding and export on a loopback cluster: a node started with `nodes`
//! joins without a bootstrap list, and dead seeds cost one retry cycle, not one per entry.

mod common;

use std::net::UdpSocket;
use std::time::{Duration, Instant};

use common::{Node, cluster, spawn_config, within};
use peeroxide_dht::hyperdht::KeyPair;
use peeroxide_dht::messages::Ipv4Peer;
use peeroxide_dht::rpc::DhtConfig;

const K: usize = 20;

fn config(bootstrap: Vec<String>, nodes: Vec<Ipv4Peer>) -> DhtConfig {
    let mut dht = DhtConfig::default();
    dht.host = "127.0.0.1".to_string();
    dht.firewalled = false;
    dht.bootstrap = bootstrap;
    dht.nodes = nodes;
    dht
}

fn key() -> KeyPair {
    KeyPair::from_seed([0x5a; 32])
}

/// Holds `count` bound UDP ports that never answer.
fn dead_ports(count: usize) -> (Vec<UdpSocket>, Vec<Ipv4Peer>) {
    let socks: Vec<UdpSocket> = (0..count)
        .map(|_| UdpSocket::bind("127.0.0.1:0").expect("bind"))
        .collect();
    let peers = socks
        .iter()
        .map(|s| Ipv4Peer {
            host: "127.0.0.1".to_string(),
            port: s.local_addr().expect("addr").port(),
        })
        .collect();
    (socks, peers)
}

async fn cluster_ports(nodes: &[Node]) -> Vec<u16> {
    let mut ports = Vec::new();
    for n in nodes {
        ports.push(n.handle.local_port().await.expect("port"));
    }
    ports
}

async fn found(node: &Node) -> bool {
    let kp = key();
    match node.handle.mutable_get(&kp.public_key, 0).await {
        Ok(Some(r)) => r.value == b"saved-nodes",
        _ => false,
    }
}

#[tokio::test]
async fn seeded_node_joins_and_finds_record() {
    within(async {
        let (nodes, _bs) = cluster(4).await;
        nodes[1]
            .handle
            .mutable_put(&key(), b"saved-nodes", 1)
            .await
            .expect("put");
        let seeds = nodes[2].handle.to_array(None).await.expect("export");
        assert!(!seeds.is_empty());
        let node = spawn_config(config(vec![], seeds)).await;
        assert!(node.handle.dht().table_size().await.expect("size") > 0);
        let mode = node.handle.mode();
        assert!(
            mode.ephemeral && mode.adaptive,
            "joins as ephemeral: {mode:?}"
        );
        assert!(found(&node).await, "record found through seeded table");
    })
    .await;
}

#[tokio::test]
async fn unseeded_node_stays_alone() {
    within(async {
        let (nodes, _bs) = cluster(4).await;
        nodes[1]
            .handle
            .mutable_put(&key(), b"saved-nodes", 1)
            .await
            .expect("put");
        let node = spawn_config(config(vec![], vec![])).await;
        assert_eq!(node.handle.dht().table_size().await.expect("size"), 0);
        assert!(!found(&node).await);
    })
    .await;
}

#[tokio::test]
async fn export_lists_live_neighbours() {
    within(async {
        let (nodes, _bs) = cluster(3).await;
        let ports = cluster_ports(&nodes).await;
        let list = nodes[0].handle.to_array(None).await.expect("export");
        assert!(!list.is_empty());
        for (i, p) in list.iter().enumerate() {
            assert_eq!(p.host, "127.0.0.1");
            assert!(ports.contains(&p.port), "{} is not a cluster port", p.port);
            assert!(list[..i].iter().all(|q| q.port != p.port), "duplicate");
        }
        assert_eq!(
            nodes[0].handle.to_array(Some(1)).await.expect("one").len(),
            1
        );
    })
    .await;
}

/// Spawns one arm with `dead` unanswering seeds; returns the join time.
async fn arm(bs: &[String], dead: usize, ports: &[u16], check_stale: bool) -> Duration {
    let (_socks, peers) = dead_ports(dead);
    let start = Instant::now();
    let node = within(spawn_config(config(bs.to_vec(), peers.clone()))).await;
    let took = start.elapsed();
    assert!(node.handle.dht().table_size().await.expect("size") > 0);
    assert!(found(&node).await, "record found with {dead} dead seeds");
    if !check_stale {
        return took;
    }
    let list = node.handle.to_array(None).await.expect("export");
    let stale = list
        .iter()
        .filter(|p| peers.iter().any(|d| d.port == p.port) && !ports.contains(&p.port))
        .count();
    let allowed = dead.saturating_sub(K);
    assert!(
        stale <= allowed,
        "{stale} dead seeds left with {dead} seeded"
    );
    took
}

fn mean(xs: &[Duration]) -> f64 {
    xs.iter().map(Duration::as_secs_f64).sum::<f64>() / xs.len() as f64
}

fn secs(xs: &[Duration]) -> Vec<String> {
    xs.iter()
        .map(|d| format!("{:.2}", d.as_secs_f64()))
        .collect()
}

#[tokio::test]
async fn dead_seeds_cost_does_not_grow_with_count() {
    let (nodes, bs) = cluster(3).await;
    nodes[1]
        .handle
        .mutable_put(&key(), b"saved-nodes", 1)
        .await
        .expect("put");
    let ports = cluster_ports(&nodes).await;
    let mut t = Vec::new();
    for dead in [0, 2, 24, 24, 2, 0] {
        t.push(arm(&bs, dead, &ports, true).await);
    }
    let few = mean(&[t[1], t[4]]);
    let many = mean(&[t[2], t[3]]);
    let msg = format!("none/few/many/many/few/none = {:?} s", secs(&t));
    println!("{msg}");
    assert!(many < 2.0 * few, "many {many:.2} vs few {few:.2}: {msg}");
}

#[tokio::test]
async fn dead_seeds_cost_one_timer_when_k_answer() {
    let (nodes, bs) = cluster(22).await;
    nodes[1]
        .handle
        .mutable_put(&key(), b"saved-nodes", 1)
        .await
        .expect("put");
    let ports = cluster_ports(&nodes).await;
    let mut t = Vec::new();
    for dead in [0, 24, 24, 0] {
        t.push(arm(&bs, dead, &ports, false).await);
    }
    let diff = mean(&[t[1], t[2]]) - mean(&[t[0], t[3]]);
    let msg = format!("none/many/many/none = {:?} s, diff {diff:.2}", secs(&t));
    println!("{msg}");
    assert!(diff < 3.0, "{msg}");
}
