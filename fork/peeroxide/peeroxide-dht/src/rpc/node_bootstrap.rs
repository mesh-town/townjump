// JS: dht-rpc/index.js
//! `impl DhtNode`: bootstrap top-up for queries (js `query.js` `_open`: bootstrap nodes join the queue
//! when it holds fewer than k nodes). Neighbours: `node_query` (user queries), `node_tick` (refresh).

use crate::query::{Query, parse_bootstrap_str};

use super::node::DhtNode;

impl DhtNode {
    /// Parsed `host:port` pairs of the configured bootstrap nodes.
    pub(super) fn bootstrap_nodes(&self) -> Vec<(String, u16)> {
        self.config
            .bootstrap
            .iter()
            .filter_map(|s| parse_bootstrap_str(s))
            .collect()
    }

    /// Call right after `Query::add_from_table`: a short queue (e.g. an empty table) gets the bootstrap nodes.
    pub(super) fn top_up_from_bootstrap(&self, q: &mut Query) {
        q.add_nodes_if_sparse(&self.bootstrap_nodes());
    }
}

#[cfg(test)]
#[path = "node_bootstrap_tests.rs"]
mod tests;
