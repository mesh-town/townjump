// JS: dht-rpc/index.js (constructor opts.nodes, addNode, _addNode, toArray)
//! Seed nodes: entries of `DhtConfig::nodes` go into the routing table at start without a ping,
//! and the table can be exported as `{host, port}` pairs (JS `toArray`).

use std::collections::HashSet;
use std::net::Ipv4Addr;

use crate::messages::Ipv4Peer;
use crate::peer::{NodeId, peer_id};
use crate::routing_table::{Node, RoutingTable};

use super::node::DhtNode;

/// The node's own bound address; entries pointing at it are dropped.
pub(super) struct Own<'a> {
    pub host: &'a str,
    pub port: u16,
}

fn is_own(addr: Ipv4Addr, entry: &Ipv4Peer, own: &Own<'_>) -> bool {
    if entry.port != own.port {
        return false;
    }
    addr.is_loopback() || addr.is_unspecified() || entry.host == own.host
}

/// Builds a table entry for a seed address, or `None` when it must be dropped silently.
pub(super) fn seed_node(
    entry: &Ipv4Peer,
    table_id: &NodeId,
    own: &Own<'_>,
    tick: u64,
) -> Option<Node> {
    let addr: Ipv4Addr = entry.host.parse().ok()?;
    if entry.port == 0 || is_own(addr, entry, own) {
        return None;
    }
    let id = peer_id(&entry.host, entry.port);
    if id == *table_id {
        return None;
    }
    Some(Node {
        id,
        host: entry.host.clone(),
        port: entry.port,
        token: None,
        added_tick: tick,
        seen_tick: tick,
        pinged_tick: tick,
        down_hints: 0,
    })
}

/// Inserts valid entries in input order; returns how many went in.
/// A full bucket drops the entry, and its `BucketFull` events are discarded.
pub(super) fn seed_table(
    table: &mut RoutingTable,
    entries: &[Ipv4Peer],
    own: &Own<'_>,
    tick: u64,
) -> usize {
    let table_id = *table.id();
    let mut inserted = 0;
    for entry in entries {
        let Some(node) = seed_node(entry, &table_id, own, tick) else {
            continue;
        };
        if table.has(&node.id) || !table.add(node) {
            continue;
        }
        inserted += 1;
    }
    table.drain_events();
    inserted
}

/// Table entries as host/port pairs, freshest first; `None` means all, `Some(0)` none.
pub(super) fn export(table: &RoutingTable, limit: Option<usize>) -> Vec<Ipv4Peer> {
    let mut all: Vec<&Node> = table.iter().collect();
    all.sort_by_key(|n| std::cmp::Reverse(n.seen_tick));
    let mut seen = HashSet::new();
    all.into_iter()
        .filter(|n| seen.insert(n.id))
        .take(limit.unwrap_or(usize::MAX))
        .map(|n| Ipv4Peer {
            host: n.host.clone(),
            port: n.port,
        })
        .collect()
}

impl DhtNode {
    /// Puts `config.nodes` into the table before the bootstrap query starts.
    pub(super) fn add_seed_nodes(&mut self) {
        if self.config.nodes.is_empty() {
            return;
        }
        let Ok(mut table) = self.table.lock() else {
            return;
        };
        let own = Own {
            host: &self.config.host,
            port: self.local_port,
        };
        seed_table(&mut table, &self.config.nodes, &own, self.tick);
    }
}

#[cfg(test)]
#[path = "node_seeds_tests.rs"]
mod tests;
