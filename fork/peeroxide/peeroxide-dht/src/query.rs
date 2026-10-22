//! Iterative DHT query engine.
//!
//! Faithful Rust port of `dht-rpc/lib/query.js`.
//! A `Query` is a state machine driven by the DHT node event loop.

#![allow(missing_docs)]
#![allow(dead_code)]
#![deny(clippy::all)]

use std::collections::{HashMap, HashSet};
use std::net::Ipv4Addr;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{mpsc, oneshot};

use crate::io::RequestParams;
use crate::messages::{Command, Ipv4Peer};
use crate::peer::{NodeId, peer_id};
use crate::routing_table::{K, RoutingTable};

#[path = "query_values.rs"]
mod values;

// ── Constants ─────────────────────────────────────────────────────────────────

const DOWN_HINT_CMD: u64 = Command::DownHint as u64;

// ── Public types ──────────────────────────────────────────────────────────────

/// A node entry in the query's pending list.
#[derive(Debug, Clone)]
pub(crate) struct QueryNode {
    pub id: NodeId,
    pub host: String,
    pub port: u16,
}

/// A reply from a node that made it into the `closest_replies` set.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct QueryReply {
    /// The peer that replied.
    pub from: Ipv4Peer,
    /// Our address as seen by the responder (`msg.to`); `Some` only when the
    /// responder reported a usable address (non-empty host, non-zero port).
    pub to: Option<Ipv4Peer>,
    /// Validated node ID (non-`None` only if the peer proved its address).
    pub from_id: Option<NodeId>,
    /// Round-trip token the peer echoed back (for commit operations).
    pub token: Option<[u8; 32]>,
    /// Closer nodes suggested by the responder.
    pub closer_nodes: Vec<Ipv4Peer>,
    /// Error code (0 = success).
    pub error: u64,
    /// Response value payload.
    pub value: Option<Vec<u8>>,
    /// Measured round-trip time for this reply.
    pub rtt: Duration,
}

/// The final result of a completed query.
#[derive(Debug)]
#[non_exhaustive]
pub struct QueryResult {
    /// Up to K closest nodes found, sorted by XOR distance to target.
    pub closest_replies: Vec<QueryReply>,
    /// Number of successful replies received.
    pub successes: u32,
    /// Number of error / timeout replies.
    pub errors: u32,
}

/// Response data forwarded from the IO layer to the query engine.
#[derive(Debug)]
pub(crate) struct IoResponseData {
    pub from: Ipv4Peer,
    /// Address the request went to; may differ from `from` (reply from another address of the
    /// same node). `seen`/`slow_addrs` are keyed by it.
    pub(crate) sent_to: Ipv4Peer,
    pub(crate) to: Ipv4Peer,
    pub from_id: Option<NodeId>,
    pub token: Option<[u8; 32]>,
    pub closer_nodes: Vec<Ipv4Peer>,
    pub error: u64,
    pub value: Option<Vec<u8>>,
    pub rtt: Duration,
}

/// A request produced by the query engine for the [`crate::rpc::DhtNode`] to send.
#[derive(Debug)]
pub(crate) enum QueryRequest {
    /// Phase-1 iterative query request — the DhtNode must map the resulting TID
    /// back to this query so responses are routed here.
    Query(RequestParams),
    /// Phase-2 commit request — route response/timeout completion back via
    /// [`Query::on_commit_done`].
    Commit(RequestParams),
    /// DOWN_HINT fire-and-forget — do **not** map the TID to this query.
    DownHint(RequestParams),
}

// ── Private types ─────────────────────────────────────────────────────────────

#[derive(Debug)]
enum SeenState {
    /// Node is in the pending list; the `Vec` holds referrers.
    Pending(Vec<Ipv4Peer>),
    /// Node responded successfully (or with an application error).
    Done,
    /// Node timed out — considered unreachable.
    Down,
}

// ── Query ─────────────────────────────────────────────────────────────────────

/// Iterative Kademlia query state machine.
///
/// Driven externally by [`crate::rpc::DhtNode`]:
/// - Call [`Query::poll_requests`] after creation (and after adding nodes) to
///   get the initial set of requests to send.
/// - Call [`Query::on_response`] / [`Query::on_timeout`] as IO events arrive;
///   [`Query::on_cycle`] — when a request's first timer expired (js `_slow`).
/// - Call [`Query::on_commit_done`] when a commit-phase TID is resolved.
/// - Check [`Query::is_finished`] after each call; if true, the result has been
///   sent via the oneshot channel and the query may be dropped.
pub(crate) struct Query {
    /// Unique query ID — used by the DhtNode to route TIDs.
    #[allow(dead_code)]
    pub id: u64,
    /// Query target (32-byte hash).
    pub target: NodeId,
    internal: bool,
    command: u64,
    value: Option<Vec<u8>>,
    k: usize,
    /// Concurrency limit (mutable — callers may lower/raise it).
    pub concurrency: usize,
    commit: bool,

    inflight: usize,
    slowdown: bool,
    /// Successful replies received.
    pub successes: u32,
    /// Error / timeout replies received.
    pub errors: u32,
    from_table: bool,
    committing: bool,
    commit_inflight: usize,

    seen: HashMap<String, SeenState>,
    /// Addresses of search requests whose first timer expired; parity with js `_slow`.
    slow_addrs: HashSet<String>,
    pending: Vec<QueryNode>,
    /// Closest replies sorted by XOR distance; at most K entries.
    pub closest_replies: Vec<QueryReply>,

    finished: bool,
    result_tx: Option<oneshot::Sender<QueryResult>>,
    table: Arc<Mutex<RoutingTable>>,
    own_id: NodeId,
    /// Session (`DhtHandle::session`) that owns this query; its destroy removes the query.
    pub(crate) session: Option<u64>,
    /// Listener of every successful reply that carries a value (js `_onvisit` -> `push`).
    pub(crate) values_tx: Option<mpsc::UnboundedSender<QueryReply>>,
}

impl Query {
    /// Create a new iterative query.
    ///
    /// Pass `retries = u32::MAX` to use the default (5 for queries, 3 for
    /// DOWN_HINT).  The routing table reference is used for the fallback
    /// `add_from_table` path.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        id: u64,
        target: NodeId,
        internal: bool,
        command: u64,
        value: Option<Vec<u8>>,
        concurrency: usize,
        commit: bool,
        result_tx: oneshot::Sender<QueryResult>,
        table: Arc<Mutex<RoutingTable>>,
        own_id: NodeId,
    ) -> Self {
        Self {
            id,
            target,
            internal,
            command,
            value,
            k: K,
            concurrency,
            commit,
            inflight: 0,
            slowdown: false,
            successes: 0,
            errors: 0,
            from_table: false,
            committing: false,
            commit_inflight: 0,
            seen: HashMap::new(),
            slow_addrs: HashSet::new(),
            pending: Vec::new(),
            closest_replies: Vec::new(),
            finished: false,
            result_tx: Some(result_tx),
            table,
            own_id,
            session: None,
            values_tx: None,
        }
    }

    /// Populate the pending list from the routing table's closest nodes.
    pub(crate) fn add_from_table(&mut self) {
        if self.pending.len() >= self.k {
            return;
        }
        self.from_table = true;
        let need = self.k.saturating_sub(self.pending.len());
        let nodes = {
            let table = match self.table.lock() {
                Ok(t) => t,
                Err(_) => return,
            };
            table
                .closest(&self.target, need)
                .into_iter()
                .map(|n| QueryNode {
                    id: n.id,
                    host: n.host.clone(),
                    port: n.port,
                })
                .collect::<Vec<_>>()
        };
        for node in nodes {
            self.add_pending_impl(node, None);
        }
    }

    /// Add explicit nodes (e.g., bootstrap or user-supplied nodes) to pending.
    pub(crate) fn add_nodes(&mut self, nodes: &[(String, u16)]) {
        for (host, port) in nodes {
            let id = peer_id(host, *port);
            if id == self.own_id {
                continue;
            }
            self.add_pending_impl(
                QueryNode {
                    id,
                    host: host.clone(),
                    port: *port,
                },
                None,
            );
        }
    }

    /// Generate the initial set of requests to send.
    /// Call once after [`Query::add_from_table`] / [`Query::add_nodes`].
    pub(crate) fn poll_requests(&mut self) -> Vec<QueryRequest> {
        self.read_more()
    }

    /// Handle an IO response for a phase-1 TID belonging to this query.
    /// Returns additional requests to send (phase-1 or commit, plus DOWN_HINTs).
    pub(crate) fn on_response(&mut self, data: IoResponseData) -> Vec<QueryRequest> {
        if self.inflight > 0 {
            self.inflight -= 1;
        }

        let addr = format!("{}:{}", data.sent_to.host, data.sent_to.port);
        self.slow_addrs.remove(&addr);
        self.seen.insert(addr, SeenState::Done);

        if self.committing {
            return vec![];
        }

        if data.error == 0 {
            self.successes += 1;
        } else {
            self.errors += 1;
        }

        // Add to closest_replies if the peer proved its ID and it's closer.
        if data.error == 0 {
            if let Some(id) = data.from_id {
                if self.is_closer(&id) {
                    self.push_closest(values::reply_of(&data));
                }
            }
        }
        self.emit_value(&data);

        // Add suggested closer nodes.  Stop when one is not closer (JS break).
        let mut extra: Vec<QueryRequest> = Vec::new();
        for node in &data.closer_nodes {
            let node_id = peer_id(&node.host, node.port);
            if node_id == self.own_id {
                continue;
            }
            let qn = QueryNode {
                id: node_id,
                host: node.host.clone(),
                port: node.port,
            };
            let (should_break, hint) = self.add_pending_returning(qn, Some(data.from.clone()));
            if let Some(h) = hint {
                extra.push(h);
            }
            if should_break {
                break;
            }
        }

        // End initial slowdown once we have enough results.
        if !self.from_table && self.successes + self.errors >= self.concurrency as u32 {
            self.slowdown = false;
        }

        extra.extend(self.read_more());
        extra
    }

    /// Handle a timeout for a phase-1 TID belonging to this query.
    /// Returns DOWN_HINT requests plus next phase-1 requests.
    pub(crate) fn on_timeout(&mut self, to: &Ipv4Peer) -> Vec<QueryRequest> {
        let addr = format!("{}:{}", to.host, to.port);

        // Collect referrers before overwriting the state.
        let refs: Vec<Ipv4Peer> = match self.seen.get(&addr) {
            Some(SeenState::Pending(refs)) => refs.clone(),
            _ => vec![],
        };

        self.slow_addrs.remove(&addr);
        self.seen.insert(addr, SeenState::Down);

        // Build DOWN_HINT requests to each referrer.
        let mut requests: Vec<QueryRequest> = refs
            .iter()
            .filter_map(|referrer| make_down_hint(referrer, to))
            .collect();

        if self.inflight > 0 {
            self.inflight -= 1;
        }
        self.errors += 1;

        if !self.committing {
            requests.extend(self.read_more());
        }

        requests
    }

    /// Called when a commit-phase request completes (response or timeout).
    /// Returns `true` if the query is now fully finished.
    pub(crate) fn on_commit_done(&mut self) -> bool {
        if self.commit_inflight > 0 {
            self.commit_inflight -= 1;
        }
        if self.commit_inflight == 0 {
            self.finish_internal();
            return true;
        }
        false
    }

    /// Returns `true` if this query has completed and the result was sent.
    pub(crate) fn is_finished(&self) -> bool {
        self.finished
    }

    /// The first timer of a request to `to` expired: the address is considered slow, the search
    /// continues without it (js `oncycle` → `_slow++`). A retransmission or a foreign address gives `[]`.
    pub(crate) fn on_cycle(&mut self, to: &Ipv4Peer) -> Vec<QueryRequest> {
        let addr = format!("{}:{}", to.host, to.port);
        if !matches!(self.seen.get(&addr), Some(SeenState::Pending(_))) {
            return vec![];
        }
        if !self.slow_addrs.insert(addr) {
            return vec![];
        }
        self.read_more()
    }

    /// Number of slow requests among inflight.
    pub(crate) fn slow(&self) -> usize {
        self.slow_addrs.len()
    }

    // ── Private helpers ───────────────────────────────────────────────────────

    /// The search may finish: nothing is in flight, or all in-flight requests are slow
    /// and the K closest replies are already there (js `_readMore`).
    fn can_finish(&self) -> bool {
        self.inflight == 0 || (self.slow() == self.inflight && self.closest_replies.len() >= self.k)
    }

    /// Core "read more" driver — mirrors JS `_readMore()`.
    fn read_more(&mut self) -> Vec<QueryRequest> {
        if self.finished || self.committing {
            return vec![];
        }

        // Effective concurrency: in slowdown mode at most 3; slow
        // requests do not take slots (js `concurrency + _slow`).
        let base = if self.slowdown {
            3_usize.min(self.concurrency)
        } else {
            self.concurrency
        };
        let effective_concurrency = base + self.slow();

        let mut requests = Vec::new();

        while self.inflight < effective_concurrency && !self.pending.is_empty() {
            let next = match self.pending.pop() {
                Some(n) => n,
                None => break,
            };
            // Skip nodes that are no longer in the closer set.
            if !self.is_closer(&next.id) {
                continue;
            }
            self.inflight += 1;
            requests.push(QueryRequest::Query(RequestParams {
                to: Ipv4Peer {
                    host: next.host,
                    port: next.port,
                },
                token: None,
                internal: self.internal,
                command: self.command,
                target: Some(self.target),
                value: self.value.clone(),
                timeout_ms: None,
                retries: None,
                node_id: Some(next.id),
            }));
        }

        // Start initial slowdown: wait for some results before going broad.
        if !self.from_table && self.successes == 0 && self.errors == 0 {
            self.slowdown = true;
        }

        if !self.pending.is_empty() {
            return requests;
        }

        // All pending drained — check finish condition.
        if self.can_finish() {
            // Fallback: if we got very few successes and haven't queried the
            // routing table yet, try populating from it.
            if !self.from_table && self.successes < self.k as u32 / 4 {
                self.add_from_table();
                if !self.pending.is_empty() {
                    requests.extend(self.read_more());
                    return requests;
                }
            }
            let flush_reqs = self.flush();
            requests.extend(flush_reqs);
        }

        requests
    }

    /// Transition to the flush / commit phase.  Mirrors JS `_flush()`.
    fn flush(&mut self) -> Vec<QueryRequest> {
        if self.committing {
            return vec![];
        }
        self.committing = true;

        if !self.commit {
            self.finish_internal();
            return vec![];
        }

        // Phase-2: re-request the closest nodes that have a valid token.
        let commit_params: Vec<RequestParams> = self
            .closest_replies
            .iter()
            .filter_map(|reply| {
                reply.token.map(|token| RequestParams {
                    to: reply.from.clone(),
                    token: Some(token),
                    internal: self.internal,
                    command: self.command,
                    target: Some(self.target),
                    value: self.value.clone(),
                    timeout_ms: None,
                    retries: None,
                    node_id: reply.from_id,
                })
            })
            .collect();

        self.commit_inflight = commit_params.len();

        if self.commit_inflight == 0 {
            // No closest nodes had tokens — finish immediately.
            self.finish_internal();
            return vec![];
        }

        commit_params
            .into_iter()
            .map(QueryRequest::Commit)
            .collect()
    }

    /// Mark the query as done and deliver the result via the oneshot channel.
    fn finish_internal(&mut self) {
        if self.finished {
            return;
        }
        self.finished = true;
        self.values_tx = None;
        if let Some(tx) = self.result_tx.take() {
            let _ = tx.send(QueryResult {
                closest_replies: self.closest_replies.clone(),
                successes: self.successes,
                errors: self.errors,
            });
        }
    }

    /// Returns `true` if `id` is closer to `target` than the farthest entry in
    /// `closest_replies`, or if the set is not yet full.
    fn is_closer(&self, id: &NodeId) -> bool {
        if self.closest_replies.len() < self.k {
            return true;
        }
        let last = &self.closest_replies[self.closest_replies.len() - 1];
        match last.from_id {
            Some(last_id) => self.compare(id, &last_id) < 0,
            None => true,
        }
    }

    /// Signed XOR-distance comparison relative to `self.target`.
    ///
    /// Returns a negative number if `a` is closer, positive if `b` is closer,
    /// zero if equidistant.  Matches JS `_compare`.
    pub(crate) fn compare(&self, a: &NodeId, b: &NodeId) -> i32 {
        for i in 0..32 {
            if a[i] == b[i] {
                continue;
            }
            let t = self.target[i];
            return (t ^ a[i]) as i32 - (t ^ b[i]) as i32;
        }
        0
    }

    /// Insertion-sort `reply` into `closest_replies`, cap at K, dedup on
    /// distance==0.  Matches JS `_pushClosest`.
    fn push_closest(&mut self, reply: QueryReply) {
        let m_id = match reply.from_id {
            Some(id) => id,
            None => return,
        };
        self.closest_replies.push(reply);
        let len = self.closest_replies.len();
        let mut i = len as isize - 2;
        while i >= 0 {
            let prev_id = match self.closest_replies[i as usize].from_id {
                Some(id) => id,
                None => break,
            };
            let cmp = self.compare(&prev_id, &m_id);
            if cmp < 0 {
                // prev is closer → m is already in the right position.
                break;
            }
            if cmp == 0 {
                // Duplicate distance — remove the newly appended copy.
                self.closest_replies.remove(i as usize + 1);
                return;
            }
            // prev is farther — bubble m up one more position.
            self.closest_replies.swap(i as usize, i as usize + 1);
            i -= 1;
        }
        if self.closest_replies.len() > self.k {
            self.closest_replies.pop();
        }
    }

    /// Add `node` to `pending`.
    ///
    /// Returns `(should_break, Option<DOWN_HINT request>)`.
    /// `should_break = true` means the node was not closer and the caller
    /// should stop processing more nodes from this response's closer_nodes list.
    fn add_pending_returning(
        &mut self,
        node: QueryNode,
        referrer: Option<Ipv4Peer>,
    ) -> (bool, Option<QueryRequest>) {
        let addr = format!("{}:{}", node.host, node.port);
        let is_closer = self.is_closer(&node.id);

        match self.seen.get(&addr) {
            Some(SeenState::Done) => {
                return (!is_closer, None);
            }
            Some(SeenState::Down) => {
                let hint = if let Some(ref ref_node) = referrer {
                    make_down_hint(
                        ref_node,
                        &Ipv4Peer {
                            host: node.host,
                            port: node.port,
                        },
                    )
                } else {
                    None
                };
                return (!is_closer, hint);
            }
            Some(SeenState::Pending(_)) => {
                if let Some(ref_node) = referrer {
                    if let Some(SeenState::Pending(refs)) = self.seen.get_mut(&addr) {
                        refs.push(ref_node);
                    }
                }
                return (!is_closer, None);
            }
            None => {}
        }

        if !is_closer {
            return (true, None);
        }

        let refs = match referrer {
            Some(r) => vec![r],
            None => vec![],
        };
        self.seen.insert(addr, SeenState::Pending(refs));
        self.pending.push(node);
        (false, None)
    }

    /// Convenience wrapper that discards the `should_break` signal.
    fn add_pending_impl(&mut self, node: QueryNode, referrer: Option<Ipv4Peer>) {
        let _ = self.add_pending_returning(node, referrer);
    }
}

// ── Free helpers ──────────────────────────────────────────────────────────────

/// Encode an IPv4 peer as 6 bytes (4 IP + 2 port LE) for DOWN_HINT payloads.
fn encode_ipv4_peer_6(peer: &Ipv4Peer) -> Option<Vec<u8>> {
    let addr = Ipv4Addr::from_str(&peer.host).ok()?;
    let mut out = Vec::with_capacity(6);
    out.extend_from_slice(&addr.octets());
    out.extend_from_slice(&peer.port.to_le_bytes());
    Some(out)
}

/// Build a DOWN_HINT `RequestParams` directed at `to`, encoding `down` as value.
fn make_down_hint(to: &Ipv4Peer, down: &Ipv4Peer) -> Option<QueryRequest> {
    let value = encode_ipv4_peer_6(down)?;
    Some(QueryRequest::DownHint(RequestParams {
        to: to.clone(),
        token: None,
        internal: true,
        command: DOWN_HINT_CMD,
        target: None,
        value: Some(value),
        timeout_ms: None,
        retries: None,
        node_id: None,
    }))
}

pub(crate) fn parse_bootstrap_str(s: &str) -> Option<(String, u16)> {
    let last_colon = s.rfind(':')?;
    let host_part = &s[..last_colon];
    let port: u16 = s[last_colon + 1..].parse().ok()?;
    // Handle "suggestedIP@hostname" format — extract the IP before '@',
    // matching Node.js dht-rpc parseNode() behavior.
    let host = if let Some(at_pos) = host_part.find('@') {
        &host_part[..at_pos]
    } else {
        host_part
    };
    Some((host.to_string(), port))
}

/// Resolve bootstrap strings to `ip:port` form.
///
/// For each entry, tries the suggested IP (before `@`) first. If absent or
/// not a valid IP, falls back to async DNS resolution of the hostname.
pub(crate) async fn resolve_bootstrap_nodes(raw: &[String]) -> Vec<String> {
    let mut resolved = Vec::with_capacity(raw.len());
    for s in raw {
        if let Some((host, port)) = resolve_one_bootstrap(s).await {
            resolved.push(format!("{host}:{port}"));
        }
    }
    resolved
}

async fn resolve_one_bootstrap(s: &str) -> Option<(String, u16)> {
    let last_colon = s.rfind(':')?;
    let host_part = &s[..last_colon];
    let port: u16 = s[last_colon + 1..].parse().ok()?;

    if let Some(at_pos) = host_part.find('@') {
        let ip = &host_part[..at_pos];
        let hostname = &host_part[at_pos + 1..];

        if ip.parse::<std::net::IpAddr>().is_ok() {
            return Some((ip.to_string(), port));
        }

        dns_resolve(hostname, port).await
    } else if host_part.parse::<std::net::IpAddr>().is_ok() {
        Some((host_part.to_string(), port))
    } else {
        dns_resolve(host_part, port).await
    }
}

async fn dns_resolve(hostname: &str, port: u16) -> Option<(String, u16)> {
    match tokio::net::lookup_host(format!("{hostname}:{port}")).await {
        Ok(mut addrs) => {
            let addr = addrs.find(|a| a.is_ipv4()).or_else(|| addrs.next())?;
            tracing::debug!(%hostname, %addr, "resolved bootstrap node via DNS");
            Some((addr.ip().to_string(), addr.port()))
        }
        Err(e) => {
            tracing::warn!(%hostname, err = %e, "DNS resolution failed for bootstrap node");
            None
        }
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "query_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "query_slow_tests.rs"]
mod slow_tests;

#[cfg(test)]
#[path = "query_reply_addr_tests.rs"]
mod reply_addr_tests;
