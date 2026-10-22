// JS: dht-rpc/index.js
//! `impl DhtNode`: the periodic tick, pinging old nodes, routing-table refresh.
//! Neighbours: `node` runs the loop; `node_table` adds and evicts nodes.

use std::sync::Arc;

use tokio::sync::oneshot;
use tokio::time::Instant;

use crate::io::RequestParams;
use crate::messages::Ipv4Peer;
use crate::peer::NodeId;
use crate::query::{Query, QueryResult};
use crate::routing_table::RoutingTable;

use super::consts::{CMD_FIND_NODE, CMD_PING, RECENT_NODE, REFRESH_TICKS, SLEEPING_INTERVAL_MS};
use super::node::DhtNode;
use super::requests::StandaloneRequest;

impl DhtNode {
    pub(super) fn handle_tick(&mut self) {
        if self.is_suspended() {
            return;
        }

        let now = Instant::now();
        let elapsed = now.duration_since(self.last_tick_time).as_millis() as u64;

        if elapsed > SLEEPING_INTERVAL_MS {
            self.wake_up();
        } else {
            self.tick += 1;
        }

        self.last_tick_time = now;
        self.down_hints_per_tick = 0;

        if !self.bootstrapped {
            return;
        }

        // Before pinging, as in JS: a promotion rebuilds the table id first.
        self.adaptive_tick();

        if (self.tick & 7) == 0 {
            self.ping_some();
        }

        self.refresh_ticks = self.refresh_ticks.saturating_sub(1);
        if self.refresh_ticks == 0 {
            self.refresh_ticks = REFRESH_TICKS;
            self.run_refresh();
        }
    }

    fn ping_some(&mut self) {
        let busy = !self.standalone_tids.is_empty();

        let nodes: Vec<(NodeId, String, u16, u64)> = {
            let table = match self.table.lock() {
                Ok(t) => t,
                Err(_) => return,
            };
            if table.is_empty() {
                drop(table);
                self.run_refresh();
                return;
            }
            pick_ping_targets(&table, self.tick, busy)
        };

        for (node_id, host, port, last_seen) in nodes {
            if let Ok(mut table) = self.table.lock() {
                if let Some(node) = table.get_mut(&node_id) {
                    node.pinged_tick = self.tick;
                }
            }
            let params = RequestParams {
                to: Ipv4Peer { host, port },
                token: None,
                internal: true,
                command: CMD_PING,
                target: None,
                value: None,
                timeout_ms: None,
                retries: None,
                node_id: None,
            };
            if let Some(tid) = self.io.create_request(params) {
                self.standalone_tids.insert(
                    tid,
                    StandaloneRequest::Check {
                        node_id,
                        last_seen_tick: last_seen,
                    },
                );
            }
        }
    }

    pub(super) fn run_refresh(&mut self) {
        self.refresh_ticks = REFRESH_TICKS;

        let target = {
            let table = match self.table.lock() {
                Ok(t) => t,
                Err(_) => return,
            };
            table.random().map(|n| n.id).unwrap_or_else(|| *table.id())
        };

        let own_id = {
            let table = match self.table.lock() {
                Ok(t) => t,
                Err(_) => return,
            };
            *table.id()
        };

        let (result_tx, result_rx) = oneshot::channel::<QueryResult>();
        let query_id = self.next_query_id;
        self.next_query_id += 1;

        let concurrency = (self.config.concurrency / 8).max(2);

        let mut q = Query::new(
            query_id,
            target,
            true,
            CMD_FIND_NODE,
            None,
            concurrency,
            false,
            result_tx,
            Arc::clone(&self.table),
            own_id,
        );
        q.add_from_table();

        let reqs = q.poll_requests();
        self.dispatch_query_requests(query_id, reqs);
        self.active_queries.insert(query_id, q);

        tokio::spawn(async move {
            let _ = result_rx.await;
        });
    }
}

/// Picks nodes for a ping check across the whole table (js `_pingSome`): the least recently
/// pinged, then the least recently seen; nodes pinged in this tick are skipped.
/// If the oldest candidate was pinged recently, 2 are taken, otherwise 3 (under load) or 5.
fn pick_ping_targets(
    table: &RoutingTable,
    tick: u64,
    busy: bool,
) -> Vec<(NodeId, String, u16, u64)> {
    let mut nodes: Vec<_> = table
        .closest(table.id(), table.len())
        .into_iter()
        .filter(|n| n.pinged_tick < tick)
        .collect();
    nodes.sort_by_key(|n| (n.pinged_tick, n.seen_tick));
    let mut cnt = if busy { 3 } else { 5 };
    if let Some(oldest) = nodes.first()
        && tick.saturating_sub(oldest.pinged_tick) < RECENT_NODE
    {
        cnt = 2;
    }
    nodes
        .into_iter()
        .take(cnt)
        .map(|n| (n.id, n.host.clone(), n.port, n.seen_tick))
        .collect()
}

#[cfg(test)]
#[path = "node_tick_tests.rs"]
mod tests;
