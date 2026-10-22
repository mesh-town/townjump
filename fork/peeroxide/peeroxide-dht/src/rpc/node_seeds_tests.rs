// JS: dht-rpc/index.js (constructor opts.nodes, addNode, _addNode, toArray)
//! Seed-node insertion and table export on a bare `RoutingTable` (no sockets).

use std::collections::HashSet;

use super::*;

const TABLE_ID: [u8; 32] = [0u8; 32];

fn own() -> Own<'static> {
    Own {
        host: "127.0.0.1",
        port: 40000,
    }
}

fn peer(host: &str, port: u16) -> Ipv4Peer {
    Ipv4Peer {
        host: host.to_string(),
        port,
    }
}

fn node_at(host: &str, port: u16, seen: u64) -> Node {
    Node {
        id: peer_id(host, port),
        host: host.to_string(),
        port,
        token: None,
        added_tick: 0,
        seen_tick: seen,
        pinged_tick: 0,
        down_hints: 0,
    }
}

mod seed_node {
    use super::*;

    #[test]
    fn valid_entry_gets_id_from_address() {
        let node = seed_node(&peer("1.2.3.4", 5), &TABLE_ID, &own(), 7).expect("valid");
        assert_eq!(node.id, peer_id("1.2.3.4", 5));
        assert_eq!((node.host.as_str(), node.port), ("1.2.3.4", 5));
        assert_eq!(
            (node.added_tick, node.seen_tick, node.pinged_tick),
            (7, 7, 7)
        );
        assert!(node.token.is_none());
        assert_eq!(node.down_hints, 0);
    }

    #[test]
    fn hostname_is_dropped() {
        for host in ["example.com", "::1", ""] {
            assert!(seed_node(&peer(host, 5), &TABLE_ID, &own(), 0).is_none());
        }
    }

    #[test]
    fn port_zero_is_dropped() {
        assert!(seed_node(&peer("1.2.3.4", 0), &TABLE_ID, &own(), 0).is_none());
    }

    #[test]
    fn own_table_id_is_dropped() {
        let id = peer_id("1.2.3.4", 5);
        assert!(seed_node(&peer("1.2.3.4", 5), &id, &own(), 0).is_none());
    }

    #[test]
    fn own_address_is_dropped() {
        assert!(seed_node(&peer("127.0.0.1", 40000), &TABLE_ID, &own(), 0).is_none());
        assert!(seed_node(&peer("0.0.0.0", 40000), &TABLE_ID, &own(), 0).is_none());
        assert!(seed_node(&peer("127.0.0.1", 40001), &TABLE_ID, &own(), 0).is_some());
    }
}

mod seed_table {
    use super::*;

    #[test]
    fn inserts_without_ping() {
        let mut table = RoutingTable::new(TABLE_ID);
        let entries = [peer("1.1.1.1", 1), peer("2.2.2.2", 2), peer("3.3.3.3", 3)];
        assert_eq!(seed_table(&mut table, &entries, &own(), 0), 3);
        assert_eq!(table.len(), 3);
        assert!(table.drain_events().is_empty());
    }

    #[test]
    fn duplicates_inserted_once() {
        let mut table = RoutingTable::new(TABLE_ID);
        let (a, b) = (peer("1.1.1.1", 1), peer("2.2.2.2", 2));
        assert_eq!(seed_table(&mut table, &[a.clone(), a, b], &own(), 0), 2);
        assert_eq!(table.len(), 2);
    }

    #[test]
    fn invalid_entries_are_skipped_not_fatal() {
        let mut table = RoutingTable::new(TABLE_ID);
        let (a, b) = (peer("1.1.1.1", 1), peer("2.2.2.2", 2));
        let entries = [
            peer("example.com", 9),
            a.clone(),
            peer("3.3.3.3", 0),
            b.clone(),
        ];
        assert_eq!(seed_table(&mut table, &entries, &own(), 0), 2);
        assert!(table.has(&peer_id(&a.host, a.port)));
        assert!(table.has(&peer_id(&b.host, b.port)));
    }

    #[test]
    fn overflow_is_dropped_without_events() {
        let mut table = RoutingTable::new(TABLE_ID);
        let entries: Vec<Ipv4Peer> = (1..=200u32)
            .map(|i| peer(&format!("10.0.0.{i}"), 1000))
            .collect();
        let inserted = seed_table(&mut table, &entries, &own(), 0);
        assert!(inserted < 200, "inserted {inserted}");
        assert_eq!(table.len(), inserted);
        assert!(table.drain_events().is_empty());
    }
}

mod export {
    use super::*;

    fn table_with_seen(seen: &[u64]) -> RoutingTable {
        let mut table = RoutingTable::new(TABLE_ID);
        for (i, s) in seen.iter().enumerate() {
            assert!(table.add(node_at(&format!("5.5.5.{}", i + 1), 7, *s)));
        }
        table
    }

    #[test]
    fn empty_table_gives_empty() {
        assert!(export(&RoutingTable::new(TABLE_ID), None).is_empty());
    }

    #[test]
    fn freshest_first() {
        let table = table_with_seen(&[5, 1, 3]);
        let hosts: Vec<String> = export(&table, None).into_iter().map(|p| p.host).collect();
        assert_eq!(hosts, ["5.5.5.1", "5.5.5.3", "5.5.5.2"]);
    }

    #[test]
    fn limit_zero() {
        assert!(export(&table_with_seen(&[5, 1, 3]), Some(0)).is_empty());
    }

    #[test]
    fn limit_one() {
        let out = export(&table_with_seen(&[5, 1, 3]), Some(1));
        assert_eq!(out, [peer("5.5.5.1", 7)]);
    }

    #[test]
    fn limit_above_size() {
        assert_eq!(export(&table_with_seen(&[5, 1, 3]), Some(10)).len(), 3);
    }

    #[test]
    fn only_host_and_port() {
        let mut table = RoutingTable::new(TABLE_ID);
        let entries = [peer("1.1.1.1", 1), peer("2.2.2.2", 2), peer("3.3.3.3", 3)];
        seed_table(&mut table, &entries, &own(), 0);
        let out: HashSet<(String, u16)> = export(&table, None)
            .into_iter()
            .map(|p| (p.host, p.port))
            .collect();
        let want: HashSet<(String, u16)> =
            entries.iter().map(|p| (p.host.clone(), p.port)).collect();
        assert_eq!(out, want);
    }
}
