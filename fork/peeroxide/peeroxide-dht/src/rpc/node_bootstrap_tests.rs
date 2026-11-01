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

const UNRESOLVED: &str = "bootstrap.invalid:9100";

#[tokio::test(start_paused = true)]
async fn unresolved_bootstrap_is_kept_and_resolved_on_top_up() {
    // given: a name that does not resolve at build time
    let mut r = rig(&[UNRESOLVED]).await;
    assert_eq!(r.node.bootstrap_unresolved, [UNRESOLVED]);
    assert!(r.node.config.bootstrap.is_empty());
    // when: a query starts; it launches one background resolution
    user_query(&mut r.node);
    assert!(r.node.bootstrap_resolving);
    // and: the resolution result arrives (fed by hand, no real DNS)
    r.node
        .handle_bootstrap_resolved(vec![(UNRESOLVED.into(), Some(boot_addr()))]);
    // then: the address is a bootstrap node, the name is no longer pending
    assert_eq!(r.node.config.bootstrap, [boot_addr()]);
    assert!(r.node.bootstrap_unresolved.is_empty());
    assert!(!r.node.bootstrap_resolving);
}

#[tokio::test(start_paused = true)]
async fn failed_resolution_keeps_the_name_for_the_next_try() {
    // given: a resolution is running
    let mut r = rig(&[UNRESOLVED]).await;
    user_query(&mut r.node);
    // when: it fails
    r.node
        .handle_bootstrap_resolved(vec![(UNRESOLVED.into(), None)]);
    // then: the name stays, a new round may start
    assert_eq!(r.node.bootstrap_unresolved, [UNRESOLVED]);
    assert!(!r.node.bootstrap_resolving);
}

#[tokio::test(start_paused = true)]
async fn resolved_bootstrap_with_empty_table_triggers_refresh() {
    // given: empty table, unresolved name, no query so far
    let mut r = rig(&[UNRESOLVED]).await;
    assert!(r.node.tid_to_query.is_empty());
    // when: the name resolves
    r.node
        .handle_bootstrap_resolved(vec![(UNRESOLVED.into(), Some(boot_addr()))]);
    // then: a refresh query went to the new bootstrap node
    assert_eq!(r.node.tid_to_query.len(), 1);
    assert!(only_query(&r.node).has_seen(&boot_addr()));
}

#[tokio::test(start_paused = true)]
async fn resolved_bootstrap_with_filled_table_does_not_refresh() {
    // given: a node in the table
    let mut r = rig(&[UNRESOLVED]).await;
    fill_table(&r.node, 9200..9201);
    // when: the name resolves
    r.node
        .handle_bootstrap_resolved(vec![(UNRESOLVED.into(), Some(boot_addr()))]);
    // then: no query is started
    assert!(r.node.tid_to_query.is_empty());
}

#[tokio::test]
async fn emptied_table_reaches_bootstrap_that_comes_up_later() {
    use std::time::Duration;
    use tokio::net::UdpSocket;
    // given: the bootstrap port is closed; the table held a node that was dropped
    let port = {
        let probe = UdpSocket::bind((HOST, 0)).await.expect("bind probe");
        probe.local_addr().expect("addr").port()
    };
    let mut r = rig(&[&format!("{HOST}:{port}")]).await;
    fill_table(&r.node, 9200..9201);
    let gone = peer_id(HOST, 9200);
    r.node.table.lock().expect("table").remove(&gone);
    // when: a responder comes up on the bootstrap port and a query starts
    let responder = UdpSocket::bind((HOST, port)).await.expect("bind responder");
    user_query(&mut r.node);
    // then: the request reaches it
    let mut buf = [0u8; 2048];
    let got = tokio::time::timeout(Duration::from_secs(5), responder.recv_from(&mut buf)).await;
    assert!(matches!(got, Ok(Ok((n, _))) if n > 0), "{got:?}");
}

#[tokio::test(start_paused = true)]
async fn one_background_resolution_at_a_time_and_its_result_arrives() {
    use std::time::Duration;
    use tokio::time::timeout;
    // given: a literal address that is still listed as unresolved (resolves without DNS)
    let mut r = rig(&[]).await;
    r.node.bootstrap_unresolved = vec![boot_addr()];
    // when: two queries start in a row
    user_query(&mut r.node);
    user_query(&mut r.node);
    // then: exactly one result arrives on the channel
    let rx = &mut r.node.bootstrap_resolved_rx;
    let done = timeout(Duration::from_secs(1), rx.recv()).await;
    let Ok(Some(done)) = done else {
        panic!("no result: {done:?}")
    };
    assert_eq!(done, [(boot_addr(), Some(boot_addr()))]);
    assert!(timeout(Duration::from_secs(1), rx.recv()).await.is_err());
    // and: handling it joins the address to the bootstrap list and frees the flag
    r.node.handle_bootstrap_resolved(done);
    assert_eq!(r.node.config.bootstrap, [boot_addr()]);
    assert!(!r.node.bootstrap_resolving);
}

#[tokio::test(start_paused = true)]
async fn start_query_with_full_table_does_not_resolve_names() {
    // given: a full table and a name waiting for resolution
    let mut r = rig(&[]).await;
    fill_table(&r.node, 9200..9200 + K as u16);
    r.node.bootstrap_unresolved = vec![boot_addr()];
    // when
    user_query(&mut r.node);
    // then: no resolution started
    assert!(!r.node.bootstrap_resolving);
    assert!(r.node.bootstrap_resolved_rx.try_recv().is_err());
}

#[tokio::test]
async fn background_resolution_reaches_bootstrap_through_the_loop() {
    use std::time::Duration;
    use tokio::net::UdpSocket;
    // given: a responder; the node has no bootstrap, only an unresolved entry for the responder
    let responder = UdpSocket::bind((HOST, 0)).await.expect("bind responder");
    let port = responder.local_addr().expect("addr").port();
    let Rig {
        mut node,
        _handle: handle,
        _rt,
    } = rig(&[]).await;
    node.bootstrap_unresolved = vec![format!("{HOST}:{port}")];
    let _loop = tokio::spawn(node.run());
    // when: one query goes through the handle (the table and bootstrap list are empty)
    let ask = tokio::spawn(async move { handle.find_node([7u8; 32]).await });
    // then: only resolution -> channel -> loop -> refresh can reach the responder
    let mut buf = [0u8; 2048];
    let got = tokio::time::timeout(Duration::from_secs(5), responder.recv_from(&mut buf)).await;
    assert!(matches!(got, Ok(Ok((n, _))) if n > 0), "{got:?}");
    ask.abort();
}
