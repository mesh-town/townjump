use super::*;

fn make_node(id: NodeId) -> Node {
    Node {
        id,
        host: "127.0.0.1".to_string(),
        port: 8080,
        token: None,
        added_tick: 0,
        seen_tick: 0,
        pinged_tick: 0,
        down_hints: 0,
    }
}

/// Build a NodeId from a local ID of [0;32] such that it lands in
/// a known bucket. For local_id = [0;32]:
///   byte[0] = 0x80 >> shift  →  bucket = shift  (0..=7)
///   For i-th byte (i>0): set byte[i] = 0x80 >> shift  →  bucket = i*8 + shift
fn node_id_for_bucket(bucket: usize) -> NodeId {
    let byte_idx = bucket / 8;
    let bit_shift = bucket % 8;
    let mut id = [0u8; 32];
    id[byte_idx] = 0x80u8 >> bit_shift;
    id
}

#[test]
fn test_new_table() {
    let local: NodeId = [0u8; 32];
    let rt = RoutingTable::new(local);
    assert_eq!(rt.id(), &local);
    assert_eq!(rt.len(), 0);
    assert!(rt.is_empty());
}

#[test]
fn test_add_single_node() {
    let local: NodeId = [0u8; 32];
    let mut rt = RoutingTable::new(local);

    let node_id = node_id_for_bucket(1);
    let node = make_node(node_id);

    assert!(rt.add(node));
    assert_eq!(rt.len(), 1);
    assert!(!rt.is_empty());
    assert!(rt.has(&node_id));
    assert!(rt.get(&node_id).is_some());
    assert_eq!(rt.get(&node_id).unwrap().id, node_id);
}

#[test]
fn test_add_duplicate_replaces() {
    let local: NodeId = [0u8; 32];
    let mut rt = RoutingTable::new(local);

    let node_id = node_id_for_bucket(1);

    let mut node1 = make_node(node_id);
    node1.port = 1111;
    rt.add(node1);

    let mut node2 = make_node(node_id);
    node2.port = 2222;
    let added = rt.add(node2);

    assert!(added);
    assert_eq!(rt.len(), 1);
    assert_eq!(rt.get(&node_id).unwrap().port, 2222);
}

#[test]
fn test_add_fills_bucket() {
    let local: NodeId = [0u8; 32];
    let mut rt = RoutingTable::new(local);

    for i in 0..K {
        let mut id = [0u8; 32];
        id[0] = 0x80;
        id[31] = i as u8;
        assert!(rt.add(make_node(id)));
    }

    assert_eq!(rt.len(), K);
    for i in 0..K {
        let mut id = [0u8; 32];
        id[0] = 0x80;
        id[31] = i as u8;
        assert!(rt.has(&id));
    }
}

#[test]
fn test_bucket_full_event() {
    let local: NodeId = [0u8; 32];
    let mut rt = RoutingTable::new(local);

    for i in 0..K {
        let mut id = [0u8; 32];
        id[0] = 0x80;
        id[31] = i as u8;
        assert!(rt.add(make_node(id)));
    }

    let mut overflow_id = [0u8; 32];
    overflow_id[0] = 0x80;
    overflow_id[31] = K as u8;
    let added = rt.add(make_node(overflow_id));

    assert!(!added);
    assert_eq!(rt.len(), K);

    let events = rt.drain_events();
    assert_eq!(events.len(), 1);
    match &events[0] {
        TableEvent::BucketFull {
            new_node,
            bucket_index,
        } => {
            assert_eq!(new_node.id, overflow_id);
            assert_eq!(*bucket_index, 0);
        }
    }

    assert!(rt.drain_events().is_empty());
}

#[test]
fn test_remove_node() {
    let local: NodeId = [0u8; 32];
    let mut rt = RoutingTable::new(local);

    let id = node_id_for_bucket(2);
    rt.add(make_node(id));
    assert_eq!(rt.len(), 1);

    let removed = rt.remove(&id);
    assert!(removed.is_some());
    assert_eq!(removed.unwrap().id, id);
    assert_eq!(rt.len(), 0);
    assert!(!rt.has(&id));

    assert!(rt.remove(&id).is_none());
}

#[test]
fn test_closest_basic() {
    let local: NodeId = [0u8; 32];
    let mut rt = RoutingTable::new(local);

    let id_b0 = node_id_for_bucket(0);
    let id_b1 = node_id_for_bucket(1);
    let id_b2 = node_id_for_bucket(2);

    rt.add(make_node(id_b0));
    rt.add(make_node(id_b1));
    rt.add(make_node(id_b2));

    let result = rt.closest(&id_b0, 3);
    assert_eq!(result.len(), 3);
    assert_eq!(result[0].id, id_b0);
    assert_eq!(result[1].id, id_b1);
    assert_eq!(result[2].id, id_b2);
}

#[test]
fn test_closest_wraps_buckets() {
    let local: NodeId = [0u8; 32];
    let mut rt = RoutingTable::new(local);

    let id_b3 = node_id_for_bucket(3);
    let id_b5 = node_id_for_bucket(5);

    rt.add(make_node(id_b3));
    rt.add(make_node(id_b5));

    let target = node_id_for_bucket(4);

    let result = rt.closest(&target, 10);
    assert_eq!(result.len(), 2);

    let ids: Vec<NodeId> = result.iter().map(|n| n.id).collect();
    assert!(ids.contains(&id_b3));
    assert!(ids.contains(&id_b5));
}

#[test]
fn test_bucket_index_self() {
    let local: NodeId = [0x42u8; 32];
    let rt = RoutingTable::new(local);
    assert_eq!(rt.bucket_index(&local), NUM_BUCKETS - 1);
}

#[test]
fn test_bucket_index_max_distance() {
    let local: NodeId = [0u8; 32];
    let rt = RoutingTable::new(local);

    let mut far_id = [0u8; 32];
    far_id[0] = 0x80;
    assert_eq!(rt.bucket_index(&far_id), 0);
}

#[test]
fn test_random_empty() {
    let rt = RoutingTable::new([0u8; 32]);
    assert!(rt.random().is_none());
}

#[test]
fn test_random_nonempty() {
    let local: NodeId = [0u8; 32];
    let mut rt = RoutingTable::new(local);

    let id = node_id_for_bucket(1);
    rt.add(make_node(id));

    let r = rt.random();
    assert!(r.is_some());
    assert_eq!(r.unwrap().id, id);
}

#[test]
fn test_get_mut() {
    let local: NodeId = [0u8; 32];
    let mut rt = RoutingTable::new(local);

    let id = node_id_for_bucket(2);
    rt.add(make_node(id));

    if let Some(node) = rt.get_mut(&id) {
        node.port = 9999;
        node.down_hints = 3;
    }

    let node = rt.get(&id).unwrap();
    assert_eq!(node.port, 9999);
    assert_eq!(node.down_hints, 3);
}

#[test]
fn recent_limit_zero_returns_empty() {
    let local: NodeId = [0u8; 32];
    let mut rt = RoutingTable::new(local);
    assert_eq!(rt.recent(0).len(), 0, "empty table, limit=0 => empty");

    for i in 0..5_usize {
        let mut n = make_node(node_id_for_bucket(i));
        n.seen_tick = i as u64;
        rt.add(n);
    }
    assert_eq!(rt.recent(0).len(), 0, "populated table, limit=0 => empty");
}

#[test]
fn recent_orders_by_seen_tick_desc() {
    let local: NodeId = [0u8; 32];
    let mut rt = RoutingTable::new(local);

    for (i, tick) in [1u64, 2, 3].iter().enumerate() {
        let mut n = make_node(node_id_for_bucket(i));
        n.seen_tick = *tick;
        rt.add(n);
    }

    let result = rt.recent(3);
    assert_eq!(result.len(), 3, "expected 3 nodes");
    assert_eq!(result[0].seen_tick, 3, "newest first");
    assert_eq!(result[1].seen_tick, 2);
    assert_eq!(result[2].seen_tick, 1, "oldest last");
}

#[test]
fn recent_skips_5_newest_when_cache_geq_8() {
    let local: NodeId = [0u8; 32];
    let mut rt = RoutingTable::new(local);

    for i in 0..10_usize {
        let mut n = make_node(node_id_for_bucket(i));
        n.seen_tick = i as u64;
        rt.add(n);
    }
    assert_eq!(rt.len(), 10);

    let result = rt.recent(4);
    assert_eq!(result.len(), 4, "expected 4 nodes");
    assert_eq!(result[0].seen_tick, 4, "pos 5 from newest");
    assert_eq!(result[1].seen_tick, 3);
    assert_eq!(result[2].seen_tick, 2);
    assert_eq!(result[3].seen_tick, 1, "pos 8 from newest");
}

#[test]
fn recent_caps_at_limit() {
    let local: NodeId = [0u8; 32];
    let mut rt = RoutingTable::new(local);

    for i in 0..20_usize {
        let mut n = make_node(node_id_for_bucket(i));
        n.seen_tick = i as u64;
        rt.add(n);
    }
    assert_eq!(rt.len(), 20);

    let result = rt.recent(4);
    assert_eq!(result.len(), 4, "recent(4) must cap at 4");
}

#[test]
fn test_closest_respects_k_limit() {
    let local: NodeId = [0u8; 32];
    let mut rt = RoutingTable::with_k(local, 5);

    for bucket in 0..10 {
        rt.add(make_node(node_id_for_bucket(bucket)));
    }

    let target = [0u8; 32];
    let result = rt.closest(&target, 3);
    assert!(result.len() <= 3);
}

#[test]
fn test_multiple_buckets() {
    let local: NodeId = [0u8; 32];
    let mut rt = RoutingTable::new(local);

    for bucket in 0..8 {
        rt.add(make_node(node_id_for_bucket(bucket)));
    }

    assert_eq!(rt.len(), 8);

    rt.remove(&node_id_for_bucket(3));
    rt.remove(&node_id_for_bucket(5));
    assert_eq!(rt.len(), 6);
    assert!(!rt.has(&node_id_for_bucket(3)));
    assert!(!rt.has(&node_id_for_bucket(5)));
    assert!(rt.has(&node_id_for_bucket(0)));
}
