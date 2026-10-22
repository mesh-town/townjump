// JS: dht-rpc/index.js
//! `struct DhtHandle` and its stats accessors, `bootstrapped`, `ping*`, `find_node`, `query`, `request`.
//! Neighbours: `handle_relay` (relay sends), `handle_state` (state getters), `rpc_suspend.rs` (suspend/resume).

use std::net::SocketAddr;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use tokio::sync::{mpsc, oneshot, watch};

use libudx::UdxSocket;

use crate::messages::Ipv4Peer;
use crate::peer::NodeId;
use crate::query::QueryReply;
use crate::routing_table::RoutingTable;

use super::command::DhtCommand;
use super::session::Session;
use super::types::{
    DhtError, DhtMode, PingResponse, ResponseData, UserQueryParams, UserRequestParams,
};

// ── DhtHandle (user-facing, Send + Sync + Clone) ──────────────────────────────

/// Handle for interacting with a running DHT node.
#[derive(Clone)]
pub struct DhtHandle {
    pub(super) cmd_tx: mpsc::UnboundedSender<DhtCommand>,
    pub(super) wire: crate::io::WireCounters,
    pub(super) table: Arc<Mutex<RoutingTable>>,
    pub(super) suspended: Arc<AtomicBool>,
    /// Session whose calls this handle makes (`None`: each call gets its own, see `session`).
    pub(super) session_id: Option<u64>,
    /// Current mode of the node (see `DhtNode::publish_mode`).
    pub(super) mode: watch::Receiver<DhtMode>,
}

impl DhtHandle {
    /// Snapshot of cumulative wire bytes (sent, received) since the DHT
    /// started. Counts every UDP datagram exchanged by this node, including
    /// retries, queries, replies, and relays.
    pub fn wire_stats(&self) -> (u64, u64) {
        self.wire.snapshot()
    }

    /// Borrow the shared wire-counter handle. Useful when you want a long-
    /// lived reference (e.g. for periodic sampling from a UI thread) without
    /// going through `wire_stats()` repeatedly.
    pub fn wire_counters(&self) -> crate::io::WireCounters {
        self.wire.clone()
    }

    /// Return up to `limit` recently-responsive DHT nodes from the local
    /// routing table, ordered by `seen_tick` descending. Used by
    /// [`crate::nat::Nat::auto_sample`] to pick ping targets that are
    /// likely to respond. Applies Node's skip-5-if-cache≥8 rule so the
    /// freshest nodes (probably engaged in current traffic) are not
    /// re-used as NAT-sample targets.
    pub(crate) fn recent_nodes(&self, limit: usize) -> Vec<Ipv4Peer> {
        let table = match self.table.lock() {
            Ok(t) => t,
            Err(_) => return Vec::new(),
        };
        table
            .recent(limit)
            .into_iter()
            .map(|n| Ipv4Peer {
                host: n.host.clone(),
                port: n.port,
            })
            .collect()
    }

    /// Forward inbound reply bytes from a non-`Io` socket (puncher socket)
    /// to the actor for TID matching. Used by the auto_sample forwarder
    /// task. Fire-and-forget; the actor is the source of truth for TID
    /// resolution.
    pub(crate) fn forward_inbound_reply_bytes(&self, addr: SocketAddr, data: Vec<u8>) {
        let _ = self
            .cmd_tx
            .send(DhtCommand::InboundReplyBytes { addr, data });
    }

    /// Forward a datagram received on a held (non-`Io`) socket to the actor:
    /// a request is answered from `via`, a response is matched by TID.
    /// Fire-and-forget; a closed actor is silent.
    pub(crate) fn forward_inbound_via(&self, addr: SocketAddr, data: Vec<u8>, via: UdxSocket) {
        let _ = self.cmd_tx.send(DhtCommand::InboundVia { addr, data, via });
    }

    /// Insert a node directly into the local routing table, bypassing the
    /// normal discovery/query path. Used only by unit tests that need
    /// deterministic `recent_nodes()` candidates (e.g. `Nat::auto_sample`
    /// tests) without a live bootstrap round-trip.
    #[cfg(test)]
    pub(crate) fn insert_node_for_test(&self, node: crate::routing_table::Node) {
        if let Ok(mut table) = self.table.lock() {
            table.add(node);
        }
    }
}

impl DhtHandle {
    /// Waits until the node has finished bootstrapping.
    pub async fn bootstrapped(&self) -> Result<(), DhtError> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(DhtCommand::Bootstrapped { reply_tx: tx })
            .map_err(|_| DhtError::ChannelClosed)?;
        rx.await.map_err(|_| DhtError::ChannelClosed)?
    }

    /// Sends a ping to `host:port`.
    pub async fn ping(&self, host: &str, port: u16) -> Result<PingResponse, DhtError> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(DhtCommand::Ping {
                host: host.to_string(),
                port,
                reply_tx: tx,
            })
            .map_err(|_| DhtError::ChannelClosed)?;
        rx.await.map_err(|_| DhtError::ChannelClosed)?
    }

    /// Send a one-shot ping out via the supplied `socket` (typically a
    /// puncher socket) and resolve with the reply. Used by
    /// [`crate::nat::Nat::auto_sample`] to seed reflexive NAT samples
    /// from the puncher socket's own UDP binding, since the reflexive
    /// observation must reflect that socket's NAT mapping — not the
    /// main DHT socket's. No automatic retry.
    #[allow(dead_code)] // used by Nat::auto_sample
    pub(crate) async fn ping_via_socket(
        &self,
        target: Ipv4Peer,
        socket: UdxSocket,
    ) -> Result<PingResponse, DhtError> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(DhtCommand::PingViaSocket {
                target,
                socket,
                reply_tx: tx,
            })
            .map_err(|_| DhtError::ChannelClosed)?;
        rx.await.map_err(|_| DhtError::ChannelClosed)?
    }

    /// Opens a [`Session`]: calls made through `Session::dht()` stop when it is destroyed or
    /// dropped (JS `dht.session()`).
    pub fn session(&self) -> Session {
        Session::new(self)
    }

    /// Runs a `find_node` query for `target`. Dropping the future cancels the query
    /// (on a `Session` handle — when the session ends).
    pub async fn find_node(&self, target: NodeId) -> Result<Vec<QueryReply>, DhtError> {
        match self.session_id {
            Some(_) => self.find_node_here(target).await,
            None => {
                let session = self.session();
                session.dht().find_node_here(target).await
            }
        }
    }

    async fn find_node_here(&self, target: NodeId) -> Result<Vec<QueryReply>, DhtError> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(DhtCommand::FindNode {
                target,
                session: self.session_id,
                reply_tx: tx,
            })
            .map_err(|_| DhtError::ChannelClosed)?;
        rx.await.map_err(|_| DhtError::ChannelClosed)?
    }

    /// Runs a custom DHT query. Dropping the future cancels the query
    /// (on a `Session` handle — when the session ends).
    pub async fn query(&self, params: UserQueryParams) -> Result<Vec<QueryReply>, DhtError> {
        match self.session_id {
            Some(_) => self.query_here(params).await,
            None => {
                let session = self.session();
                session.dht().query_here(params).await
            }
        }
    }

    async fn query_here(&self, params: UserQueryParams) -> Result<Vec<QueryReply>, DhtError> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(DhtCommand::Query {
                params,
                session: self.session_id,
                values: None,
                reply_tx: tx,
            })
            .map_err(|_| DhtError::ChannelClosed)?;
        rx.await.map_err(|_| DhtError::ChannelClosed)?
    }

    /// Sends a request to a remote peer. Dropping the future cancels the request
    /// (on a `Session` handle — when the session ends).
    pub async fn request(
        &self,
        params: UserRequestParams,
        host: &str,
        port: u16,
    ) -> Result<ResponseData, DhtError> {
        match self.session_id {
            Some(_) => self.request_here(params, host, port, None).await,
            None => {
                let session = self.session();
                session.dht().request_here(params, host, port, None).await
            }
        }
    }

    /// Like `request`, but sends (and retries) from the held socket `via`, without our node id.
    pub(crate) async fn request_via(
        &self,
        params: UserRequestParams,
        host: &str,
        port: u16,
        via: UdxSocket,
    ) -> Result<ResponseData, DhtError> {
        let via = Some(via);
        match self.session_id {
            Some(_) => self.request_here(params, host, port, via).await,
            None => {
                let session = self.session();
                session.dht().request_here(params, host, port, via).await
            }
        }
    }

    async fn request_here(
        &self,
        params: UserRequestParams,
        host: &str,
        port: u16,
        via: Option<UdxSocket>,
    ) -> Result<ResponseData, DhtError> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(DhtCommand::Request {
                params,
                host: host.to_string(),
                port,
                session: self.session_id,
                via,
                reply_tx: tx,
            })
            .map_err(|_| DhtError::ChannelClosed)?;
        rx.await.map_err(|_| DhtError::ChannelClosed)?
    }
}
