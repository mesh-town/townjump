// JS: dht-rpc/index.js
//! Tests for the bootstrap top-up in `start_query` and `run_refresh` (`node_bootstrap.rs`).
//! Related: `node_test_support.rs`. The node is built by `build_node` without a loop; handlers are called directly.

use libudx::UdxRuntime;
use tokio::sync::oneshot;

use super::super::node::{DhtNode, build_node};
use super::super::{DhtConfig, DhtHandle};
use crate::peer::peer_id;
use crate::routing_table::{K, Node};

const HOST: &str = "127.0.0.1";
const BOOT_PORT: u16 = 9100;

struct Rig {
    node: DhtNode,
    _handle: DhtHandle,
    _rt: UdxRuntime,
}

async fn rig(bootstrap: &[&str]) -> Rig {
    let rt = UdxRuntime::new().expect("runtime");
    let config = DhtConfig {
        bootstrap: bootstrap.iter().map(|s| s.to_string()).collect(),
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

fn boot_addr() -> String {
    format!("{HOST}:{BOOT_PORT}")
}

fn table_node(port: u16) -> Node {
    Node {
        id: peer_id(HOST, port),
        host: HOST.into(),
        port,
        token: None,
        added_tick: 0,
        seen_tick: 0,
        pinged_tick: 0,
        down_hints: 0,
    }
}

fn fill_table(node: &DhtNode, ports: std::ops::Range<u16>) {
    let mut table = node.table.lock().expect("table");
    for port in ports {
        table.add(table_node(port));
    }
}

fn user_query(node: &mut DhtNode) {
    let (tx, _rx) = oneshot::channel();
    node.start_query([7u8; 32], true, 2, None, false, None, None, None, tx);
}

/// The one live query of the node.
fn only_query(node: &DhtNode) -> &crate::query::Query {
    assert_eq!(node.active_queries.len(), 1);
    node.active_queries.values().next().expect("query")
}

#[tokio::test(start_paused = true)]
async fn start_query_with_empty_table_sends_to_bootstrap() {
    // given: empty table, one bootstrap node
    let mut r = rig(&[&boot_addr()]).await;
    // when
    user_query(&mut r.node);
    // then: the request went to the bootstrap node
    assert_eq!(r.node.tid_to_query.len(), 1);
    assert!(only_query(&r.node).has_seen(&boot_addr()));
}

#[tokio::test(start_paused = true)]
async fn run_refresh_with_empty_table_sends_to_bootstrap() {
    // given: empty table, one bootstrap node
    let mut r = rig(&[&boot_addr()]).await;
    // when
    r.node.run_refresh();
    // then: the refresh queries the bootstrap node (it used to send nothing)
    assert_eq!(r.node.tid_to_query.len(), 1);
    assert!(only_query(&r.node).has_seen(&boot_addr()));
}

#[tokio::test(start_paused = true)]
async fn start_query_with_full_table_does_not_touch_bootstrap() {
    // given: k nodes in the table, one bootstrap node
    let mut r = rig(&[&boot_addr()]).await;
    fill_table(&r.node, 9200..9200 + K as u16);
    // when
    user_query(&mut r.node);
    // then: the queue was full, the bootstrap node is not queued
    assert!(!r.node.tid_to_query.is_empty());
    assert!(!only_query(&r.node).has_seen(&boot_addr()));
}

#[tokio::test(start_paused = true)]
async fn bootstrap_already_in_table_is_asked_once() {
    // given: the bootstrap node is also the only table node
    let mut r = rig(&[&boot_addr()]).await;
    fill_table(&r.node, BOOT_PORT..BOOT_PORT + 1);
    // when
    user_query(&mut r.node);
    // then: one request, not two
    assert_eq!(r.node.tid_to_query.len(), 1);
}
