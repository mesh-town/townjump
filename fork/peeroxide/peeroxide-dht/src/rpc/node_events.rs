// JS: dht-rpc/index.js
//! `impl DhtNode`: inbound IO events, incoming requests, delayed ping, down-hint handling.
//! Neighbours: `node_responses` handles replies/timeouts; `node` has the struct and `run`.

use std::net::Ipv4Addr;
use std::time::Duration;

use crate::io::{IoEvent, RequestParams};
use crate::messages::Ipv4Peer;
use crate::peer::peer_id;
use crate::query::IoResponseData;

use super::consts::{
    CMD_DELAYED_PING, CMD_DOWN_HINT, CMD_FIND_NODE, CMD_PING, CMD_PING_NAT, ERR_UNKNOWN_COMMAND,
};
use super::node::DhtNode;
use super::requests::{DeferredReply, StandaloneRequest};
use super::types::ResponseData;

impl DhtNode {
    pub(super) fn handle_io_event(&mut self, event: IoEvent) {
        match event {
            IoEvent::IncomingRequest(req) => {
                self.add_node_from_network(req.from.clone(), req.id);
                self.handle_incoming_request(req);
            }
            IoEvent::Response {
                tid,
                from,
                id,
                token,
                closer_nodes,
                error,
                value,
                rtt,
                request,
                to,
                server_arrival,
            } => {
                self.add_node_from_network(from.clone(), id);

                // Replies to pings sent from a puncher socket carry the puncher's
                // mapping, not the node's, so they must not feed the node-level window.
                let via_puncher = matches!(
                    self.standalone_tids.get(&tid),
                    Some(StandaloneRequest::PingViaSocket(_) | StandaloneRequest::NatCheck)
                );
                if to.port != 0 && !to.host.is_empty() {
                    if self.needs_id_update {
                        self.addr_samples.push(to.clone());
                    }
                    if !via_puncher {
                        self.node_nat.sample(&to, &from, self.tick);
                    }
                }

                if self.take_nat_check_reply(tid, server_arrival, &to) {
                    return;
                }

                let data = IoResponseData {
                    from: from.clone(),
                    sent_to: request.to,
                    to: to.clone(),
                    from_id: id,
                    token,
                    closer_nodes: closer_nodes.clone(),
                    error,
                    value: value.clone(),
                    rtt,
                };

                if let Some((query_id, is_commit)) = self.tid_to_query.remove(&tid) {
                    if is_commit {
                        let finished = self
                            .active_queries
                            .get_mut(&query_id)
                            .map(|q| q.on_commit_done())
                            .unwrap_or(true);
                        if finished {
                            self.active_queries.remove(&query_id);
                            self.on_query_removed(query_id);
                        }
                    } else {
                        let reqs = self
                            .active_queries
                            .get_mut(&query_id)
                            .map(|q| q.on_response(data))
                            .unwrap_or_default();
                        self.dispatch_query_requests(query_id, reqs);
                        if self
                            .active_queries
                            .get(&query_id)
                            .map(|q| q.is_finished())
                            .unwrap_or(false)
                        {
                            self.active_queries.remove(&query_id);
                            self.on_query_removed(query_id);
                        }
                    }
                } else if let Some(standalone) = self.standalone_tids.remove(&tid) {
                    self.handle_standalone_response(
                        standalone,
                        ResponseData {
                            from,
                            id,
                            token,
                            closer_nodes,
                            error,
                            value,
                            rtt,
                            to: (to.port != 0 && !to.host.is_empty()).then(|| to.clone()),
                        },
                        to,
                    );
                }
            }
        }
    }

    fn handle_incoming_request(&mut self, req: crate::io::IncomingRequest) {
        if req.internal {
            match req.command {
                CMD_PING => {
                    self.io.send_reply(&req, 0, None);
                }
                CMD_DELAYED_PING => {
                    self.handle_delayed_ping(req);
                }
                CMD_PING_NAT => {
                    let port = match req.value.as_deref() {
                        Some([lo, hi, ..]) => u16::from_le_bytes([*lo, *hi]),
                        _ => 0,
                    };
                    if port != 0 {
                        self.reply_ping_nat(&req, port);
                    }
                }
                CMD_FIND_NODE => {
                    if req.target.is_some() {
                        self.io.send_reply(&req, 0, None);
                    }
                }
                CMD_DOWN_HINT => {
                    self.handle_down_hint_request(&req);
                    self.io.send_reply(&req, 0, None);
                }
                _ => {
                    let has_target = req.target.is_some();
                    let _ = has_target;
                    self.io.send_reply(&req, ERR_UNKNOWN_COMMAND, None);
                }
            }
        } else {
            self.forward_user_request(req);
        }
    }

    fn handle_delayed_ping(&mut self, req: crate::io::IncomingRequest) {
        let delay_ms = match &req.value {
            Some(v) if v.len() >= 4 => u32::from_le_bytes([v[0], v[1], v[2], v[3]]),
            _ => return,
        };

        if delay_ms > 10_000 {
            self.io.send_reply(&req, ERR_UNKNOWN_COMMAND, None);
            return;
        }

        let duration = Duration::from_millis(delay_ms as u64);
        let tx = self.deferred_reply_tx.clone();
        let reply = DeferredReply {
            from: req.from,
            reply_ctx: req.reply_ctx,
            tid: req.tid,
            target: req.target,
            error: 0,
            value: None,
        };
        tokio::spawn(async move {
            tokio::time::sleep(duration).await;
            let _ = tx.send(reply);
        });
    }

    fn handle_down_hint_request(&mut self, req: &crate::io::IncomingRequest) {
        let val = match &req.value {
            Some(v) if v.len() >= 6 => v.clone(),
            _ => return,
        };

        let ip_bytes: [u8; 4] = [val[0], val[1], val[2], val[3]];
        let port = u16::from_le_bytes([val[4], val[5]]);
        let host = Ipv4Addr::from(ip_bytes).to_string();
        let node_id = peer_id(&host, port);

        let (found_id, found_host, found_port, seen_tick, pinged_tick) = {
            let table = match self.table.lock() {
                Ok(t) => t,
                Err(_) => return,
            };
            if let Some(node) = table.get(&node_id) {
                (
                    node.id,
                    node.host.clone(),
                    node.port,
                    node.seen_tick,
                    node.pinged_tick,
                )
            } else {
                return;
            }
        };

        if pinged_tick < self.tick {
            if let Ok(mut table) = self.table.lock() {
                if let Some(node) = table.get_mut(&found_id) {
                    node.down_hints += 1;
                    node.pinged_tick = self.tick;
                }
            }

            let params = RequestParams {
                to: Ipv4Peer {
                    host: found_host,
                    port: found_port,
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
                self.standalone_tids.insert(
                    tid,
                    StandaloneRequest::Check {
                        node_id: found_id,
                        last_seen_tick: seen_tick,
                    },
                );
            }
        }
    }
}

#[cfg(test)]
#[path = "node_reply_addr_tests.rs"]
mod reply_addr_tests;

#[cfg(test)]
#[path = "node_nat_switch_tests.rs"]
mod nat_switch_tests;

#[cfg(test)]
#[path = "node_ping_nat_tests.rs"]
mod ping_nat_tests;
