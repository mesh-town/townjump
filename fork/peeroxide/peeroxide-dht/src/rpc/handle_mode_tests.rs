//! Tests for the mode accessors of `DhtHandle` and the `handle_tick` hook.

use std::time::Duration;

use libudx::UdxRuntime;

use super::super::DhtConfig;
use super::super::node::{build_node, spawn};
use super::super::node_test_support::spot;
use crate::messages::Ipv4Peer;

fn config(stable_ticks: u64, firewalled: bool) -> DhtConfig {
    DhtConfig {
        bootstrap: vec!["127.0.0.1:9050".into()],
        ephemeral: None,
        firewalled,
        stable_ticks,
        ..DhtConfig::default()
    }
}

#[tokio::test]
async fn fresh_node_with_bootstrap_is_ephemeral_adaptive() {
    let rt = UdxRuntime::new().expect("runtime");
    let (_task, handle) = spawn(&rt, config(2, false)).await.expect("spawn");
    let mode = handle.mode();
    assert!(mode.ephemeral);
    assert!(mode.adaptive);
}

#[tokio::test]
async fn set_adaptive_false_is_visible_in_mode_and_watch() {
    let rt = UdxRuntime::new().expect("runtime");
    let (_task, handle) = spawn(&rt, config(2, false)).await.expect("spawn");
    let mut rx = handle.watch_mode();
    handle.set_adaptive(false).await.expect("set_adaptive");
    tokio::time::timeout(Duration::from_secs(5), rx.changed())
        .await
        .expect("watch notified in time")
        .expect("sender alive");
    assert!(!rx.borrow().adaptive);
    assert!(!handle.mode().adaptive);
}

#[tokio::test]
async fn handle_tick_runs_the_adaptive_tick() {
    let rt = UdxRuntime::new().expect("runtime");
    let (mut node, handle) = build_node(&rt, config(1, false)).await.expect("build");
    node.bootstrapped = true;
    let to = Ipv4Peer {
        host: "198.51.100.7".into(),
        port: node.local_port,
    };
    (0..3).for_each(|i| {
        node.node_nat.sample(&to, &spot(20_000 + i), 0);
    });
    assert!(handle.mode().ephemeral);
    node.handle_tick();
    assert!(!handle.mode().ephemeral, "one tick must promote the node");
}
