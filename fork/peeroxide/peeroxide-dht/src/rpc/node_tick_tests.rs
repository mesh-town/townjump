// JS: dht-rpc/index.js
//! Tests for `pick_ping_targets` (node selection for `ping_some`): whole table, order, number of targets.
//! Related: `node_tick.rs`. Table: ids `[n, 0, ..]` for n = 1..=60, own id zero.

use super::super::consts::RECENT_NODE;
use super::pick_ping_targets;
use crate::peer::NodeId;
use crate::routing_table::{Node, RoutingTable};

const TICK: u64 = 100;

mod whole_table {
    use super::*;

    #[test]
    fn oldest_pinged_are_picked_even_when_farthest() {
        // given: the 5 farthest (56..=60) were pinged before everyone, but seen recently
        let table = table_of(|n| if n >= 56 { (0, 90) } else { (50, 1) });
        // when
        let mut ids = picked_ids(&table, false);
        // then: exactly they are chosen (a cap of the 50 closest would not see them)
        ids.sort_unstable();
        assert_eq!(ids, [56, 57, 58, 59, 60]);
    }

    #[test]
    fn empty_candidates_give_nothing() {
        // given: all nodes were pinged in this tick
        let table = table_of(|n| (TICK, u64::from(n)));
        // when
        let ids = picked_ids(&table, false);
        // then
        assert!(ids.is_empty(), "{ids:?}");
    }

    #[test]
    fn target_describes_its_node() {
        // given: node 1 was not seen for the longest time
        let table = table_of(|n| (0, u64::from(n)));
        // when
        let first = pick_ping_targets(&table, TICK, false).remove(0);
        // then: id, address and seen_tick of the node
        assert_eq!(first, (id_of(1), "10.0.0.1".to_string(), 4001, 1));
    }
}

mod target_count {
    use super::*;

    #[test]
    fn idle_node_picks_five_old_ones() {
        // given: all old
        let table = table_of(|n| (0, u64::from(n)));
        // when
        let ids = picked_ids(&table, false);
        // then
        assert_eq!(ids.len(), 5);
    }

    #[test]
    fn busy_node_picks_three_old_ones() {
        // given: all old, there are foreign requests in flight
        let table = table_of(|n| (0, u64::from(n)));
        // when
        let ids = picked_ids(&table, true);
        // then
        assert_eq!(ids.len(), 3);
    }

    #[test]
    fn fresh_oldest_picks_the_two_oldest() {
        // given: all pinged recently, but 50 before everyone and 30 right after ("fresh" boundary
        // for the oldest: RECENT_NODE - 1 ticks ago)
        let table = table_of(|n| match n {
            50 => (TICK - (RECENT_NODE - 1), 50),
            30 => (TICK - 2, 30),
            _ => (TICK - 1, u64::from(n)),
        });
        // when
        let ids = picked_ids(&table, false);
        // then: the two oldest by pinged_tick, the oldest first
        assert_eq!(ids, [50, 30]);
    }

    #[test]
    fn fresh_oldest_wins_over_busy() {
        // given: the same fresh oldest, but the node is busy
        let table = table_of(|n| (TICK - (RECENT_NODE - 1), u64::from(n)));
        // when
        let ids = picked_ids(&table, true);
        // then: still 2, not 3
        assert_eq!(ids.len(), 2);
    }

    #[test]
    fn stale_oldest_among_fresh_ones_picks_five() {
        // given: one node was not pinged for long (but seen recently), the rest were pinged a tick ago
        for stale in [1, 30, 60] {
            let table = table_of(|n| stale_among_fresh(n, stale));
            // when
            let ids = picked_ids(&table, false);
            // then: "freshness" is taken from the oldest, wherever it sits in the table
            assert_eq!(ids.len(), 5, "stale node {stale}: {ids:?}");
            assert_eq!(ids[0], stale, "stale node chosen first");
        }
    }

    #[test]
    fn stale_oldest_among_fresh_ones_picks_three_when_busy() {
        // given: the same stale node among fresh ones, but the node is busy
        let table = table_of(|n| stale_among_fresh(n, 30));
        // when
        let ids = picked_ids(&table, true);
        // then
        assert_eq!(ids.len(), 3, "{ids:?}");
    }

    #[test]
    fn oldest_exactly_recent_node_ticks_ago_is_not_fresh() {
        // given: the oldest was pinged exactly RECENT_NODE ticks ago
        let table = table_of(|n| (TICK - RECENT_NODE, u64::from(n)));
        // when
        let ids = picked_ids(&table, false);
        // then: no longer fresh, the usual 5
        assert_eq!(ids.len(), 5);
    }
}

mod already_pinged_this_tick {
    use super::*;

    #[test]
    fn are_not_picked() {
        // given: three stale nodes (1..=3), the other 57 pinged in this tick
        let table = table_of(|n| if n <= 3 { (0, u64::from(n)) } else { (TICK, 0) });
        // when
        let mut ids = picked_ids(&table, false);
        // then: only the three stale ones, although there could be five targets
        ids.sort_unstable();
        assert_eq!(ids, [1, 2, 3]);
    }
}

mod equal_pinged_tick {
    use super::*;

    #[test]
    fn smaller_seen_tick_goes_first() {
        // given: all pinged long ago in one tick; seen_tick decreases as n grows
        let table = table_of(|n| (0, 100 - u64::from(n)));
        // when
        let picked: Vec<(u8, u64)> = pick_ping_targets(&table, TICK, false)
            .iter()
            .map(|(id, _, _, seen)| (id[0], *seen))
            .collect();
        // then: the smaller seen_tick first
        assert_eq!(picked, [(60, 40), (59, 41), (58, 42), (57, 43), (56, 44)]);
    }
}

// ── setup and helpers ─────────────────────────────────────────────────────────

/// A table of 60 nodes; `stamp(n)` gives `(pinged_tick, seen_tick)` of the node with `id[0] == n`.
/// `k = 64`, so the densest bucket (32..=60, 29 nodes) does not overflow.
fn table_of(stamp: impl Fn(u8) -> (u64, u64)) -> RoutingTable {
    let mut table = RoutingTable::with_k([0u8; 32], 64);
    for n in 1..=60u8 {
        let (pinged_tick, seen_tick) = stamp(n);
        table.add(Node {
            id: id_of(n),
            host: format!("10.0.0.{n}"),
            port: 4000 + u16::from(n),
            token: None,
            added_tick: 0,
            seen_tick,
            pinged_tick,
            down_hints: 0,
        });
    }
    assert_eq!(table.len(), 60, "all nodes fit in the table");
    table
}

/// Node `stale` was pinged long ago but seen latest; the rest were pinged a tick ago.
fn stale_among_fresh(n: u8, stale: u8) -> (u64, u64) {
    if n == stale {
        (0, 99)
    } else {
        (TICK - 1, u64::from(n))
    }
}

fn id_of(n: u8) -> NodeId {
    let mut id = [0u8; 32];
    id[0] = n;
    id
}

/// `id[0]` of the chosen nodes in selection order.
fn picked_ids(table: &RoutingTable, busy: bool) -> Vec<u8> {
    pick_ping_targets(table, TICK, busy)
        .iter()
        .map(|(id, ..)| id[0])
        .collect()
}
