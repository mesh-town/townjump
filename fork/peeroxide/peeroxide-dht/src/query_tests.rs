// JS: dht-rpc/lib/query.js
//! Inline tests for `Query`.

use super::*;
use std::sync::{Arc, Mutex};

pub(super) fn make_table() -> Arc<Mutex<RoutingTable>> {
    Arc::new(Mutex::new(RoutingTable::new([0u8; 32])))
}

pub(super) fn make_query_with_k(
    target: NodeId,
    k: usize,
) -> (Query, oneshot::Receiver<QueryResult>) {
    let (tx, rx) = oneshot::channel();
    let table = make_table();
    let mut q = Query::new(1, target, true, 2, None, 3, false, tx, table, [0u8; 32]);
    q.k = k;
    (q, rx)
}

fn make_query(target: NodeId) -> (Query, oneshot::Receiver<QueryResult>) {
    make_query_with_k(target, K)
}

pub(super) fn peer(host: &str, port: u16) -> Ipv4Peer {
    Ipv4Peer {
        host: host.into(),
        port,
    }
}

fn reply_with_id(id: NodeId, port: u16) -> QueryReply {
    QueryReply {
        from: peer("127.0.0.1", port),
        to: None,
        from_id: Some(id),
        token: None,
        closer_nodes: vec![],
        error: 0,
        value: None,
        rtt: Duration::from_millis(5),
    }
}

// ── compare / XOR distance ────────────────────────────────────────────────

#[test]
fn test_compare_identical() {
    let target = [0u8; 32];
    let (q, _) = make_query(target);
    let a = [1u8; 32];
    assert_eq!(q.compare(&a, &a), 0);
}

#[test]
fn test_compare_a_closer() {
    // target = 0x00..., a[0]=0x01 (xor=1), b[0]=0x02 (xor=2) → a closer
    let target = [0u8; 32];
    let (q, _) = make_query(target);
    let mut a = [0u8; 32];
    a[0] = 0x01;
    let mut b = [0u8; 32];
    b[0] = 0x02;
    assert!(q.compare(&a, &b) < 0, "a should be closer than b");
}

#[test]
fn test_compare_b_closer() {
    let target = [0u8; 32];
    let (q, _) = make_query(target);
    let mut a = [0u8; 32];
    a[0] = 0x04;
    let mut b = [0u8; 32];
    b[0] = 0x01;
    assert!(q.compare(&a, &b) > 0, "b should be closer than a");
}

#[test]
fn test_compare_tiebreak_second_byte() {
    // first bytes equal, compare on second byte
    let mut target = [0u8; 32];
    target[0] = 0xFF; // same xor for first byte when a[0]=b[0]=0xFF
    let (q, _) = make_query(target);
    let mut a = [0xFFu8; 32];
    a[1] = 0x01; // xor with target[1]=0 → distance 1
    let mut b = [0xFFu8; 32];
    b[1] = 0x02; // xor → distance 2
    assert!(q.compare(&a, &b) < 0);
}

// ── push_closest (insertion sort + cap) ──────────────────────────────────

#[test]
fn test_push_closest_sorted_order() {
    let target = [0u8; 32];
    let (mut q, _) = make_query_with_k(target, 20);

    let mut id1 = [0u8; 32];
    id1[0] = 0x01;
    let mut id2 = [0u8; 32];
    id2[0] = 0x02;
    let mut id4 = [0u8; 32];
    id4[0] = 0x04;

    // Insert out of order
    q.push_closest(reply_with_id(id4, 4));
    q.push_closest(reply_with_id(id1, 1));
    q.push_closest(reply_with_id(id2, 2));

    assert_eq!(q.closest_replies.len(), 3);
    assert_eq!(q.closest_replies[0].from_id, Some(id1)); // closest first
    assert_eq!(q.closest_replies[1].from_id, Some(id2));
    assert_eq!(q.closest_replies[2].from_id, Some(id4)); // farthest last
}

#[test]
fn test_push_closest_cap_at_k() {
    let target = [0u8; 32];
    let (mut q, _) = make_query_with_k(target, 3);

    for i in 1u8..=5 {
        let mut id = [0u8; 32];
        id[0] = i * 0x10;
        q.push_closest(reply_with_id(id, i as u16));
    }
    assert_eq!(q.closest_replies.len(), 3, "capped at k=3");
}

#[test]
fn test_push_closest_dedup() {
    let target = [0u8; 32];
    let (mut q, _) = make_query_with_k(target, 20);

    let mut id = [0u8; 32];
    id[0] = 0x10;
    q.push_closest(reply_with_id(id, 1));
    q.push_closest(reply_with_id(id, 2)); // same id → duplicate
    assert_eq!(q.closest_replies.len(), 1, "duplicate should be discarded");
}

// ── is_closer ─────────────────────────────────────────────────────────────

#[test]
fn test_is_closer_empty_set() {
    let target = [0u8; 32];
    let (q, _) = make_query(target);
    assert!(
        q.is_closer(&[0xFFu8; 32]),
        "anything is closer when set is empty"
    );
}

#[test]
fn test_is_closer_full_set() {
    let target = [0u8; 32];
    let (mut q, _) = make_query_with_k(target, 2);

    let mut id1 = [0u8; 32];
    id1[0] = 0x01;
    let mut id4 = [0u8; 32];
    id4[0] = 0x04;
    q.push_closest(reply_with_id(id1, 1));
    q.push_closest(reply_with_id(id4, 2));

    let mut closer = [0u8; 32];
    closer[0] = 0x02; // xor=2 < xor(0x04)=4 → closer than farthest
    assert!(q.is_closer(&closer));

    let mut farther = [0u8; 32];
    farther[0] = 0x08; // xor=8 > 4
    assert!(!q.is_closer(&farther));
}

// ── DhtConfig defaults ────────────────────────────────────────────────────

#[test]
fn test_query_default_k_is_20() {
    let target = [0u8; 32];
    let (q, _) = make_query(target);
    assert_eq!(q.k, 20);
}

// ── add_from_table ────────────────────────────────────────────────────────

#[test]
fn test_add_from_table_populates_pending() {
    let target = [0u8; 32];
    let (tx, _rx) = oneshot::channel();
    let mut table = RoutingTable::new([0u8; 32]);

    let mut nid = [0u8; 32];
    nid[0] = 0x42;
    table.add(crate::routing_table::Node {
        id: nid,
        host: "127.0.0.1".into(),
        port: 8888,
        token: None,
        added_tick: 0,
        seen_tick: 0,
        pinged_tick: 0,
        down_hints: 0,
    });

    let table_arc = Arc::new(Mutex::new(table));
    let mut q = Query::new(
        1, target, true, 2, None, 10, false, tx, table_arc, [0u8; 32],
    );
    assert!(q.pending.is_empty());

    q.add_from_table();

    assert!(q.from_table, "from_table flag should be set");
    assert!(!q.pending.is_empty(), "pending should have the table node");
}

// ── Bootstrap node parsing ────────────────────────────────────────────────

#[test]
fn test_bootstrap_parse_simple() {
    let (host, port) = parse_bootstrap_str("192.168.1.1:7777").expect("parse");
    assert_eq!(host, "192.168.1.1");
    assert_eq!(port, 7777);
}

#[test]
fn test_bootstrap_parse_with_suggested_ip() {
    let (host, port) =
        parse_bootstrap_str("10.0.0.1@bootstrap.example.com:24242").expect("parse with @");
    assert_eq!(port, 24242);
    assert_eq!(host, "10.0.0.1");
}

#[test]
fn test_bootstrap_parse_bad() {
    assert!(parse_bootstrap_str("nocolon").is_none());
}

#[tokio::test]
async fn test_resolve_bootstrap_ip_passthrough() {
    let result = resolve_bootstrap_nodes(&["1.2.3.4:1234".to_string()]).await;
    assert_eq!(result, vec!["1.2.3.4:1234"]);
}

#[tokio::test]
async fn test_resolve_bootstrap_suggested_ip_preferred() {
    let result = resolve_bootstrap_nodes(&["10.0.0.1@example.invalid:9999".to_string()]).await;
    assert_eq!(result, vec!["10.0.0.1:9999"]);
}

#[tokio::test]
async fn test_resolve_bootstrap_hostname_only() {
    let result = resolve_bootstrap_nodes(&["localhost:5555".to_string()]).await;
    assert_eq!(result, vec!["127.0.0.1:5555"]);
}

#[tokio::test]
async fn test_resolve_bootstrap_bad_hostname_skipped() {
    let result = resolve_bootstrap_nodes(&["doesnotexist.invalid:1234".to_string()]).await;
    assert!(result.is_empty());
}
