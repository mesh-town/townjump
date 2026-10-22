// JS: dht-rpc/index.js
//! `impl DhtNode`: forwarding user requests, standalone responses, retries (`handle_retry`)
//! and timeouts (`handle_expired`, `handle_timeout`).
//! Neighbours: `node_events` dispatches into these; `requests` has the request records.

use tokio::sync::oneshot;

use crate::io::{RetryEvent, TimeoutEvent};
use crate::messages::Ipv4Peer;

use super::consts::{ERR_UNKNOWN_COMMAND, SUPPRESS_REPLY_SENTINEL};
use super::node::DhtNode;
use super::requests::{DeferredReply, StandaloneRequest, UserRequest};
use super::types::{DhtError, PingResponse, ResponseData};

impl DhtNode {
    pub(super) fn forward_user_request(&mut self, req: crate::io::IncomingRequest) {
        let (reply_tx, reply_rx) = oneshot::channel::<(u64, Option<Vec<u8>>)>();

        let user_req = UserRequest {
            from: req.from.clone(),
            id: req.id,
            tid: req.tid,
            token: req.token,
            command: req.command,
            target: req.target,
            value: req.value.clone(),
            arrived_on: self.io.arrival_socket(&req.reply_ctx),
            reply_tx: Some(reply_tx),
        };

        let mut maybe_req = Some(user_req);
        self.request_subscribers.retain(|tx| {
            if let Some(ur) = maybe_req.take() {
                match tx.send(ur) {
                    Ok(()) => true,
                    Err(e) => {
                        maybe_req = Some(e.0);
                        false
                    }
                }
            } else {
                !tx.is_closed()
            }
        });

        if maybe_req.is_some() {
            self.io.send_reply(&req, ERR_UNKNOWN_COMMAND, None);
        } else {
            let deferred_tx = self.deferred_reply_tx.clone();
            let from = req.from.clone();
            let reply_ctx = req.reply_ctx;
            let tid = req.tid;
            let target = req.target;
            tokio::spawn(async move {
                let outcome = match reply_rx.await {
                    Ok((SUPPRESS_REPLY_SENTINEL, _)) => None,
                    Ok((error, value)) => Some((error, value)),
                    Err(_) => Some((ERR_UNKNOWN_COMMAND, None)),
                };
                if let Some((error, value)) = outcome {
                    let _ = deferred_tx.send(DeferredReply {
                        from,
                        reply_ctx,
                        tid,
                        target,
                        error,
                        value,
                    });
                }
            });
        }
    }

    pub(super) fn handle_standalone_response(
        &mut self,
        standalone: StandaloneRequest,
        resp: ResponseData,
        reflexive_addr: Ipv4Peer,
    ) {
        match standalone {
            StandaloneRequest::Ping(reply_tx) | StandaloneRequest::PingViaSocket(reply_tx) => {
                let to = if reflexive_addr.port != 0 && !reflexive_addr.host.is_empty() {
                    Some(reflexive_addr)
                } else {
                    None
                };
                let _ = reply_tx.send(Ok(PingResponse {
                    from: resp.from,
                    id: resp.id,
                    rtt: resp.rtt,
                    to,
                    closer_nodes: resp.closer_nodes,
                }));
            }
            StandaloneRequest::UserRequest(reply_tx) => {
                let _ = reply_tx.send(Ok(resp));
            }
            StandaloneRequest::NatCheck => {}
            StandaloneRequest::Reping {
                new_node,
                old_node_id,
                last_seen_tick,
            } => {
                self.repinging = self.repinging.saturating_sub(1);
                let stale = {
                    let table = self.table.lock().ok();
                    table
                        .and_then(|t| t.get(&old_node_id).map(|n| n.seen_tick <= last_seen_tick))
                        .unwrap_or(true)
                };
                if stale {
                    if let Ok(mut table) = self.table.lock() {
                        table.remove(&old_node_id);
                        table.add(new_node);
                    }
                }
            }
            StandaloneRequest::Check {
                node_id,
                last_seen_tick,
            } => {
                let stale = {
                    let table = self.table.lock().ok();
                    table
                        .and_then(|t| t.get(&node_id).map(|n| n.seen_tick <= last_seen_tick))
                        .unwrap_or(false)
                };
                if stale {
                    if let Ok(mut table) = self.table.lock() {
                        table.remove(&node_id);
                    }
                }
            }
        }
    }

    /// Processes expired timers: retransmissions first, then final timeouts.
    /// The retransmission and the final of one tid come from different IO calls, so the order here is
    /// only determinism and matches js, where `oncycle` runs before `ontimeout`.
    pub(super) fn handle_expired(&mut self) {
        let (retries, timeouts) = self.io.check_timeouts_and_retries();
        for evt in retries {
            self.handle_retry(evt);
        }
        for evt in timeouts {
            self.handle_timeout(evt);
        }
    }

    /// The first timer of a search request expired: the query considers the address slow (js `_slow`).
    /// The tid stays in `tid_to_query`: it is the same request; the commit phase has no `oncycle`.
    pub(super) fn handle_retry(&mut self, evt: RetryEvent) {
        let Some(&(query_id, is_commit)) = self.tid_to_query.get(&evt.tid) else {
            return;
        };
        if is_commit {
            return;
        }
        let reqs = self
            .active_queries
            .get_mut(&query_id)
            .map(|q| q.on_cycle(&evt.to))
            .unwrap_or_default();
        self.advance_query(query_id, reqs);
    }

    pub(super) fn handle_timeout(&mut self, evt: TimeoutEvent) {
        // A search node that did not answer through all retransmissions is dropped from the table (js `_ontimeout`).
        // The lock is released before `on_timeout`: `add_from_table` takes the same `Mutex`.
        if let Some(id) = evt.node_id
            && let Ok(mut table) = self.table.lock()
        {
            table.remove(&id);
        }
        if let Some((query_id, is_commit)) = self.tid_to_query.remove(&evt.tid) {
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
                    .map(|q| q.on_timeout(&evt.to))
                    .unwrap_or_default();
                self.advance_query(query_id, reqs);
            }
        } else if let Some(standalone) = self.standalone_tids.remove(&evt.tid) {
            self.handle_standalone_timeout(standalone);
        }
    }

    fn handle_standalone_timeout(&mut self, standalone: StandaloneRequest) {
        match standalone {
            StandaloneRequest::Ping(reply_tx) | StandaloneRequest::PingViaSocket(reply_tx) => {
                let _ = reply_tx.send(Err(DhtError::RequestFailed("timeout".into())));
            }
            StandaloneRequest::UserRequest(reply_tx) => {
                let _ = reply_tx.send(Err(DhtError::RequestFailed("timeout".into())));
            }
            StandaloneRequest::NatCheck => self.on_nat_check_timeout(),
            StandaloneRequest::Reping {
                new_node,
                old_node_id,
                ..
            } => {
                self.repinging = self.repinging.saturating_sub(1);
                if let Ok(mut table) = self.table.lock() {
                    table.remove(&old_node_id);
                    table.add(new_node);
                }
            }
            StandaloneRequest::Check { node_id, .. } => {
                if let Ok(mut table) = self.table.lock() {
                    table.remove(&node_id);
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "node_responses_tests.rs"]
mod tests;
