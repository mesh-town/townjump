// JS: dht-rpc/index.js
//! Shared fixture for `DhtNode` tests: a node with B in the table and a search query to it (`setup`,
//! `Search`, `Fixture`). Tests: `node_responses_tests.rs`, `node_cancel_tests.rs`.

use std::sync::Arc;
use std::time::Duration;

use libudx::UdxRuntime;
use tokio::sync::oneshot;

use super::node::{DhtNode, build_node};
use super::requests::StandaloneRequest;
use super::{DhtConfig, DhtHandle};
use crate::io::{RetryEvent, TimeoutEvent};
use crate::messages::Ipv4Peer;
use crate::peer::{NodeId, peer_id};
use crate::query::Query;
use crate::routing_table::Node;

pub(crate) const HOST: &str = "127.0.0.1";
/// Node B: the query sent it a request, it is in the table.
pub(crate) const PORT_B: u16 = 9002;
/// Nodes A1 and A2 wait in the query `pending` (only with `Search::WithSpare`).
pub(crate) const PORT_SPARE: u16 = 9001;
pub(crate) const PORT_SPARE2: u16 = 9003;
pub(crate) const QID: u64 = 1;

// ── setup and helpers ─────────────────────────────────────────────────────────

/// What happens to the search query to node B.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Search {
    /// No query at all.
    Absent,
    /// B is the only query node: after its timeout the query finishes via `add_from_table`.
    Single,
    /// The request to B was sent, nodes A1 and A2 wait in `pending`: after a retransmission the query sends one
    /// request (to A1).
    WithSpare,
    /// The request to B was sent, but the query is already finished and removed (`on_query_removed` cancelled the request).
    Finished,
}

pub(crate) struct Fixture {
    pub(crate) node: DhtNode,
    /// tid of the query request to B (0 if there is no query).
    pub(crate) tid: u16,
    _handle: DhtHandle,
    /// The runtime is kept for the whole test; after `node`, so the sockets close first.
    _rt: UdxRuntime,
}

impl Fixture {
    pub(crate) fn has_b(&self) -> bool {
        self.node.table.lock().expect("table").has(&b_id())
    }

    /// How many tids are tracked by the query.
    pub(crate) fn tracked(&self) -> usize {
        self.node.tid_to_query.len()
    }

    /// Registers a standalone request and returns its tid.
    pub(crate) fn register(&mut self, request: StandaloneRequest) -> u16 {
        let tid = 5000;
        self.node.standalone_tids.insert(tid, request);
        tid
    }

    /// A second passed with margin (request timer is 1 s, clock stopped by `pause`),
    /// the node processed the expired timers.
    pub(crate) async fn when_second_passes(&mut self) {
        tokio::time::advance(Duration::from_millis(1100)).await;
        self.node.handle_expired();
    }

    pub(crate) async fn when_query_timeout(self, node_id: Option<NodeId>) -> Self {
        let tid = self.tid;
        self.when_timeout(tid, node_id).await
    }

    pub(crate) async fn when_timeout(self, tid: u16, node_id: Option<NodeId>) -> Self {
        let evt = TimeoutEvent {
            tid,
            to: spot(PORT_B),
            command: 1,
            internal: true,
            target: None,
            node_id,
        };
        self.guarded(move |node| node.handle_timeout(evt)).await
    }

    pub(crate) async fn when_retry(self, tid: u16) -> Self {
        let evt = RetryEvent {
            tid,
            to: spot(PORT_B),
        };
        self.guarded(move |node| node.handle_retry(evt)).await
    }

    /// The handler runs in a separate thread under a timeout: a hung `table` lock
    /// (std `Mutex` is not reentrant) fails the test instead of hanging it. The thread is not from the
    /// `spawn_blocking` pool: the runtime `Drop` waits for that pool, and the test would hang anyway.
    pub(crate) async fn guarded(self, act: impl FnOnce(&mut DhtNode) + Send + 'static) -> Self {
        let Fixture {
            mut node,
            tid,
            _handle,
            _rt,
        } = self;
        let (done_tx, done_rx) = oneshot::channel();
        let runtime = tokio::runtime::Handle::current();
        std::thread::spawn(move || {
            let _enter = runtime.enter();
            act(&mut node);
            let _ = done_tx.send(node);
        });
        let Ok(done) = tokio::time::timeout(Duration::from_secs(10), done_rx).await else {
            // Both the thread and the node sockets in it hung: the libudx runtime will not wait for them on `Drop`.
            std::mem::forget((_handle, _rt));
            panic!("handler hung: table lock not released");
        };
        Fixture {
            node: done.expect("handler crashed"),
            tid,
            _handle,
            _rt,
        }
    }
}

/// A node with B in the table and the query `search` to it.
pub(crate) async fn setup(search: Search) -> Fixture {
    let rt = UdxRuntime::new().expect("runtime");
    let config = DhtConfig {
        bootstrap: vec![],
        ephemeral: Some(true),
        ..DhtConfig::default()
    };
    let (mut node, handle) = build_node(&rt, config).await.expect("build_node");
    node.table.lock().expect("table").add(node_b());
    let tid = start_search(&mut node, search);
    Fixture {
        node,
        tid,
        _handle: handle,
        _rt: rt,
    }
}

/// Starts a query with concurrency 1 and sends a request to B; returns its tid.
pub(crate) fn start_search(node: &mut DhtNode, search: Search) -> u16 {
    if search == Search::Absent {
        return 0;
    }
    let own_id = *node.table.lock().expect("table").id();
    let (tx, _rx) = oneshot::channel();
    let table = Arc::clone(&node.table);
    let mut q = Query::new(QID, [0u8; 32], true, 2, None, 1, false, tx, table, own_id);
    if search == Search::WithSpare {
        q.add_nodes(&[(HOST.to_string(), PORT_SPARE2)]);
        q.add_nodes(&[(HOST.to_string(), PORT_SPARE)]);
    }
    q.add_nodes(&[(HOST.to_string(), PORT_B)]);
    let reqs = q.poll_requests();
    node.dispatch_query_requests(QID, reqs);
    let tid = *node.tid_to_query.keys().next().expect("request to B sent");
    if search == Search::Finished {
        // Real path: the query is finished and removed, `on_query_removed` cancels its requests.
        node.on_query_removed(QID);
    } else {
        node.active_queries.insert(QID, q);
    }
    tid
}

pub(crate) fn spot(port: u16) -> Ipv4Peer {
    Ipv4Peer {
        host: HOST.into(),
        port,
    }
}

pub(crate) fn b_id() -> NodeId {
    peer_id(HOST, PORT_B)
}

pub(crate) fn node_b() -> Node {
    Node {
        id: b_id(),
        host: HOST.into(),
        port: PORT_B,
        token: None,
        added_tick: 0,
        seen_tick: 0,
        pinged_tick: 0,
        down_hints: 0,
    }
}
