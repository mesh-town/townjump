// JS: dht-rpc/index.js
//! `impl DhtNode`: routing-table growth from the network, full buckets, bootstrap completion,
//! external-address voting. Neighbours: `node_tick` pings/evicts; `node_events` feeds nodes in.

use std::collections::HashMap;

use crate::io::RequestParams;
use crate::messages::Ipv4Peer;
use crate::peer::{NodeId, peer_id};
use crate::routing_table::{Node, TableEvent};

use super::consts::{CMD_PING, MAX_REPINGING, OLD_NODE, RECENT_NODE};
use super::node::DhtNode;
use super::requests::StandaloneRequest;

impl DhtNode {
    pub(super) fn add_node_from_network(&mut self, from: Ipv4Peer, from_id: Option<NodeId>) {
        let id = match from_id {
            Some(id) => id,
            None => return,
        };

        let own_id = {
            let table = match self.table.lock() {
                Ok(t) => t,
                Err(_) => return,
            };
            *table.id()
        };

        if id == own_id {
            return;
        }

        {
            let mut table = match self.table.lock() {
                Ok(t) => t,
                Err(_) => return,
            };
            if let Some(node) = table.get_mut(&id) {
                node.seen_tick = self.tick;
                node.pinged_tick = self.tick;
                return;
            }
        }

        let new_node = Node {
            id,
            host: from.host,
            port: from.port,
            token: None,
            added_tick: self.tick,
            seen_tick: self.tick,
            pinged_tick: self.tick,
            down_hints: 0,
        };

        let added = {
            let mut table = match self.table.lock() {
                Ok(t) => t,
                Err(_) => return,
            };
            table.add(new_node.clone())
        };

        if !added {
            self.handle_bucket_full(new_node);
        }
    }

    fn handle_bucket_full(&mut self, new_node: Node) {
        if !self.bootstrapped || self.repinging >= MAX_REPINGING {
            return;
        }

        let events: Vec<TableEvent> = {
            let mut table = match self.table.lock() {
                Ok(t) => t,
                Err(_) => return,
            };
            table.drain_events()
        };

        for evt in events {
            let TableEvent::BucketFull {
                new_node: evt_new_node,
                bucket_index: _,
            } = evt;
            {
                let oldest = {
                    let table = match self.table.lock() {
                        Ok(t) => t,
                        Err(_) => return,
                    };
                    let close = table.closest(&evt_new_node.id, 20);
                    close
                        .iter()
                        .filter(|n| n.pinged_tick < self.tick)
                        .min_by(|a, b| {
                            a.pinged_tick
                                .cmp(&b.pinged_tick)
                                .then(a.added_tick.cmp(&b.added_tick))
                        })
                        .map(|n| (n.id, n.host.clone(), n.port, n.seen_tick))
                };

                if let Some((old_id, old_host, old_port, last_seen)) = oldest {
                    if self.tick - last_seen < RECENT_NODE
                        && self.tick.saturating_sub(
                            self.table
                                .lock()
                                .ok()
                                .and_then(|t| t.get(&old_id).map(|n| n.added_tick))
                                .unwrap_or(0),
                        ) > OLD_NODE
                    {
                        return;
                    }

                    if let Ok(mut table) = self.table.lock() {
                        if let Some(node) = table.get_mut(&old_id) {
                            node.pinged_tick = self.tick;
                        }
                    }

                    let params = RequestParams {
                        to: Ipv4Peer {
                            host: old_host,
                            port: old_port,
                        },
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
                        self.repinging += 1;
                        self.standalone_tids.insert(
                            tid,
                            StandaloneRequest::Reping {
                                new_node: evt_new_node,
                                old_node_id: old_id,
                                last_seen_tick: last_seen,
                            },
                        );
                    }
                }
            }
        }

        let _ = new_node;
    }

    pub(super) fn mark_bootstrapped(&mut self) {
        if self.bootstrapped {
            return;
        }

        if self.needs_id_update {
            if let Some(addr) = self.determine_address_from_samples() {
                let new_id = peer_id(&addr.host, addr.port);
                if let Ok(mut table) = self.table.lock() {
                    table.rebuild_with_id(new_id);
                }
                tracing::debug!(host = %addr.host, port = addr.port, "updated node ID from NAT samples");
                self.node_address = Some(addr);
                self.publish_mode();
            }
            self.needs_id_update = false;
        }

        self.bootstrapped = true;
        tracing::info!(
            target: "peeroxide::_events::dht::bootstrapped",
            "DHT node bootstrapped"
        );
        for tx in self.bootstrap_waiters.drain(..) {
            let _ = tx.send(Ok(()));
        }
    }

    fn determine_address_from_samples(&self) -> Option<Ipv4Peer> {
        if self.addr_samples.is_empty() {
            return None;
        }

        let mut counts: HashMap<String, usize> = HashMap::new();
        for sample in &self.addr_samples {
            let key = format!("{}:{}", sample.host, sample.port);
            *counts.entry(key).or_default() += 1;
        }

        let best = counts.into_iter().max_by_key(|(_, count)| *count)?;
        let (host_port, _) = best;
        let parts: Vec<&str> = host_port.rsplitn(2, ':').collect();
        if parts.len() == 2 {
            let port: u16 = parts[0].parse().ok()?;
            let host = parts[1].to_string();
            Some(Ipv4Peer { host, port })
        } else {
            None
        }
    }
}
