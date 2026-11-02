//! Tests for the bounded name resolution and the gated refresh in `node_bootstrap.rs`.

use std::time::Duration;

use tokio::time::Instant;

use super::super::node::{DhtNode, build_node};
use super::super::{DhtConfig, DhtHandle};
use super::{ResolvedBootstrap, resolve_each_with};
use libudx::UdxRuntime;

const LIMIT: Duration = Duration::from_secs(10);
const NAME: &str = "bootstrap.invalid:9100";
const ADDR: &str = "127.0.0.1:9100";

async fn resolve_stuck_and_fine(names: &[&str]) -> (ResolvedBootstrap, Duration) {
    let raw: Vec<String> = names.iter().map(|s| s.to_string()).collect();
    let start = Instant::now();
    let out = tokio::time::timeout(
        3 * LIMIT,
        resolve_each_with(&raw, LIMIT, |s| async move {
            if s.starts_with("fine") {
                Some(ADDR.to_string())
            } else {
                std::future::pending().await
            }
        }),
    )
    .await
    .expect("resolution must be bounded");
    (out, start.elapsed())
}

#[tokio::test(start_paused = true)]
async fn hanging_name_yields_none_after_the_limit() {
    let (out, took) = resolve_stuck_and_fine(&["stuck:1", "fine:2"]).await;
    assert_eq!(
        out,
        [
            ("stuck:1".to_string(), None),
            ("fine:2".to_string(), Some(ADDR.to_string()))
        ]
    );
    assert!(took >= LIMIT && took < 2 * LIMIT, "{took:?}");
}

#[tokio::test(start_paused = true)]
async fn every_name_hanging_yields_all_none() {
    let (out, took) = resolve_stuck_and_fine(&["a:1", "b:2"]).await;
    assert_eq!(out, [("a:1".to_string(), None), ("b:2".to_string(), None)]);
    assert!(took >= 2 * LIMIT && took < 3 * LIMIT, "{took:?}");
}

struct Rig {
    node: DhtNode,
    _handle: DhtHandle,
    _rt: UdxRuntime,
}

async fn rig() -> Rig {
    let rt = UdxRuntime::new().expect("runtime");
    let config = DhtConfig {
        bootstrap: vec![NAME.to_string()],
        ephemeral: Some(true),
        ..DhtConfig::default()
    };
    let (node, handle) = build_node(&rt, config).await.expect("build_node");
    Rig {
        node,
        _handle: handle,
        _rt: rt,
    }
}

/// Feeds a resolution result with an empty table and checks the accounting but no refresh.
fn assert_no_refresh_but_accounted(node: &mut DhtNode) {
    assert!(node.table.lock().expect("table").is_empty());
    node.handle_bootstrap_resolved(vec![(NAME.into(), Some(ADDR.into()))]);
    assert!(node.active_queries.is_empty());
    assert_eq!(node.config.bootstrap, [ADDR]);
    assert!(node.bootstrap_unresolved.is_empty());
    assert!(!node.bootstrap_resolving);
}

#[tokio::test(start_paused = true)]
async fn resolved_bootstrap_while_suspended_does_not_refresh() {
    let mut r = rig().await;
    r.node.bootstrapped = true;
    r.node
        .suspended
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert_no_refresh_but_accounted(&mut r.node);
}

#[tokio::test(start_paused = true)]
async fn resolved_bootstrap_before_bootstrap_finished_does_not_refresh() {
    let mut r = rig().await;
    r.node.bootstrapped = false;
    assert_no_refresh_but_accounted(&mut r.node);
}
