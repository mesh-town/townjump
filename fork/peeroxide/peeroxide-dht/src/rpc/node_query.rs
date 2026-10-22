// JS: dht-rpc/index.js
//! `impl DhtNode`: starting queries, dispatching their requests, query removal, deferred replies.
//! Neighbours: `node_command` starts queries; `requests` has `DeferredReply`.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use tokio::sync::{mpsc, oneshot};

use crate::io::Owner;
use crate::peer::NodeId;
use crate::query::{Query, QueryReply, QueryRequest, QueryResult};

use super::consts::DOWN_HINTS_RATE_LIMIT;
use super::node::DhtNode;
use super::requests::{DeferredReply, StandaloneRequest};
use super::suspend::dropped_query_error;
use super::types::DhtError;

impl DhtNode {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn start_query(
        &mut self,
        target: NodeId,
        internal: bool,
        command: u64,
        value: Option<Vec<u8>>,
        commit: bool,
        concurrency: Option<usize>,
        session: Option<u64>,
        values: Option<mpsc::UnboundedSender<QueryReply>>,
        reply_tx: oneshot::Sender<Result<Vec<QueryReply>, DhtError>>,
    ) {
        let own_id = {
            let table = match self.table.lock() {
                Ok(t) => t,
                Err(_) => {
                    let _ = reply_tx.send(Err(DhtError::ChannelClosed));
                    return;
                }
            };
            *table.id()
        };

        let concurrency = concurrency.unwrap_or(self.config.concurrency);
        let (result_tx, result_rx) = oneshot::channel::<QueryResult>();
        let query_id = self.next_query_id;
        self.next_query_id += 1;

        let mut q = Query::new(
            query_id,
            target,
            internal,
            command,
            value,
            concurrency,
            commit,
            result_tx,
            Arc::clone(&self.table),
            own_id,
        );

        q.session = session;
        q.values_tx = values;
        q.add_from_table();

        let reqs = q.poll_requests();
        self.dispatch_query_requests(query_id, reqs);
        self.active_queries.insert(query_id, q);

        let suspended = Arc::clone(&self.suspended);
        tokio::spawn(async move {
            match result_rx.await {
                Ok(result) => {
                    let _ = reply_tx.send(Ok(result.closest_replies));
                }
                Err(_) => {
                    let dropped = suspended.load(Ordering::SeqCst);
                    let _ = reply_tx.send(Err(dropped_query_error(dropped)));
                }
            }
        });
    }

    pub(super) fn dispatch_query_requests(&mut self, query_id: u64, requests: Vec<QueryRequest>) {
        let owner = Some(Owner::Query(query_id));
        for req in requests {
            match req {
                QueryRequest::Query(params) => {
                    if let Some(tid) = self.io.create_request_owned(params, owner) {
                        self.tid_to_query.insert(tid, (query_id, false));
                    }
                }
                QueryRequest::Commit(params) => {
                    if let Some(tid) = self.io.create_request_owned(params, owner) {
                        self.tid_to_query.insert(tid, (query_id, true));
                    }
                }
                QueryRequest::DownHint(params) => {
                    if self.down_hints_per_tick < DOWN_HINTS_RATE_LIMIT {
                        self.down_hints_per_tick += 1;
                        let _ = self.io.create_request_owned(params, owner);
                    }
                }
            }
        }
    }

    /// Sends the query requests, then removes it from the active set if it is finished.
    pub(super) fn advance_query(&mut self, query_id: u64, reqs: Vec<QueryRequest>) {
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

    /// Query is gone (finished, aborted, dropped): its requests stop (no retry, no timeout)
    /// and its tids leave `tid_to_query`. The routing table is not touched.
    pub(super) fn on_query_removed(&mut self, query_id: u64) {
        self.io.cancel_owner(Owner::Query(query_id));
        self.tid_to_query.retain(|_, (id, _)| *id != query_id);
        if self.bootstrap_query_id == Some(query_id) {
            self.bootstrap_query_id = None;
            self.mark_bootstrapped();
        }
    }

    /// Session is gone: its requests stop, its standalone callers get `RequestFailed("destroyed")`,
    /// its queries are removed (their callers get the dropped-query error). The routing table is
    /// not touched.
    pub(super) fn destroy_session(&mut self, id: u64) {
        let tids = self.io.cancel_owner(Owner::Session(id));
        let mut requests = 0;
        for tid in tids {
            if let Some(StandaloneRequest::UserRequest(tx)) = self.standalone_tids.remove(&tid) {
                requests += 1;
                let _ = tx.send(Err(DhtError::RequestFailed("destroyed".into())));
            }
        }
        let doomed: Vec<u64> = self
            .active_queries
            .iter()
            .filter(|(_, q)| q.session == Some(id))
            .map(|(qid, _)| *qid)
            .collect();
        for qid in &doomed {
            self.active_queries.remove(qid);
            self.on_query_removed(*qid);
        }
        let table_len = self.table.lock().map_or(0, |t| t.len());
        tracing::debug!(
            session = id,
            requests,
            queries = doomed.len(),
            table_len,
            "destroy_session"
        );
    }

    pub(super) fn handle_deferred_reply(&mut self, reply: DeferredReply) {
        self.io.send_reply_deferred(
            &reply.from,
            reply.reply_ctx,
            reply.tid,
            reply.target,
            reply.error,
            reply.value.as_deref(),
        );
    }
}

#[cfg(test)]
#[path = "node_cancel_tests.rs"]
mod cancel_tests;
