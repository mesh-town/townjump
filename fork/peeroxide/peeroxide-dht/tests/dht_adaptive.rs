//! Adaptive mode on a loopback cluster: a node starts ephemeral and promotes itself
//! (JS dht-rpc `_updateNetworkState`). Each test spans a few 5 s ticks.

mod common;

use std::time::Duration;

use common::{Node, cluster, spawn_adaptive, spawn_config, within};
use peeroxide_dht::peer::peer_id;
use peeroxide_dht::rpc::{DhtConfig, DhtMode};

async fn wait_mode(node: &Node, want: impl Fn(&DhtMode) -> bool) -> DhtMode {
    let mut rx = node.handle.watch_mode();
    let seen = rx.wait_for(|m| want(m)).await.expect("node alive");
    seen.clone()
}

/// Spawns a firewalled adaptive node and waits until it is persistent.
async fn promoted_firewalled(bs: &[String]) -> Node {
    let node = spawn_adaptive(bs.to_vec(), true, 2).await;
    wait_mode(&node, |m| !m.ephemeral).await;
    node
}

#[tokio::test]
async fn open_node_promotes() {
    within(async {
        let (_cluster, bs) = cluster(3).await;
        let node = spawn_adaptive(bs, false, 2).await;
        let start = node.handle.mode();
        assert!(start.ephemeral && start.adaptive);
        let mode = wait_mode(&node, |m| !m.ephemeral).await;
        let addr = mode.address.expect("persistent node has an address");
        let id = node.handle.dht().table_id().await.expect("table id");
        assert_eq!(id, Some(peer_id(&addr.host, addr.port)));
    })
    .await;
}

#[tokio::test]
async fn firewalled_node_promotes_via_ping_nat() {
    within(async {
        let (_cluster, bs) = cluster(6).await;
        let node = spawn_adaptive(bs, true, 2).await;
        assert!(node.handle.mode().firewalled);
        let mode = wait_mode(&node, |m| !m.ephemeral).await;
        assert!(
            !mode.firewalled,
            "a proven-open port stops being firewalled"
        );
    })
    .await;
}

#[tokio::test]
async fn unreachable_node_stays_ephemeral() {
    within(async {
        let (cluster, bs) = cluster(4).await;
        let first = spawn_adaptive(bs.clone(), true, 2).await;
        // Promotes on its sampled address alone (no PING_NAT), after 4 ticks: later than the
        // whole check round of `first` (2 ticks plus the request timeouts).
        let control = spawn_adaptive(bs, false, 4).await;
        // Nobody answers PING_NAT from here on, so the check of `first` can only time out.
        for node in &cluster {
            node.handle.suspend().await.expect("suspend");
        }
        wait_mode(&control, |m| !m.ephemeral).await;
        let mode = first.handle.mode();
        assert!(mode.ephemeral, "an unanswered check must not promote");
        assert!(mode.firewalled);
    })
    .await;
}

#[tokio::test]
async fn wakeup_demotes_and_restores_firewalled() {
    within(async {
        let (_cluster, bs) = cluster(6).await;
        let node = promoted_firewalled(&bs).await;
        assert!(!node.handle.mode().firewalled);
        node.handle.suspend().await.expect("suspend");
        node.handle.resume().await.expect("resume");
        let mode = wait_mode(&node, |m| m.ephemeral && m.firewalled).await;
        assert_eq!(mode.address, None);
    })
    .await;
}

#[tokio::test]
async fn set_adaptive_toggles_the_mode() {
    within(async {
        let (_cluster, bs) = cluster(3).await;
        let node = spawn_adaptive(bs, false, 2).await;
        wait_mode(&node, |m| !m.ephemeral).await;
        node.handle.set_adaptive(false).await.expect("off");
        let off = wait_mode(&node, |m| m.ephemeral).await;
        assert!(!off.adaptive);
        node.handle.set_adaptive(true).await.expect("on");
        wait_mode(&node, |m| !m.ephemeral && m.adaptive).await;
    })
    .await;
}

#[tokio::test]
async fn explicit_ephemeral_never_promotes() {
    within(async {
        let (_cluster, bs) = cluster(3).await;
        let mut dht = DhtConfig::default();
        dht.bootstrap = bs;
        dht.host = "127.0.0.1".to_string();
        dht.firewalled = false;
        dht.ephemeral = Some(true);
        dht.stable_ticks = 2;
        let node = spawn_config(dht).await;
        let mut rx = node.handle.watch_mode();
        rx.borrow_and_update();
        // Three ticks are well past the two that would promote an adaptive node.
        let changed = tokio::time::timeout(Duration::from_secs(16), rx.changed()).await;
        assert!(changed.is_err(), "mode changed: {:?}", node.handle.mode());
        let mode = node.handle.mode();
        assert!(mode.ephemeral && !mode.adaptive);
    })
    .await;
}
