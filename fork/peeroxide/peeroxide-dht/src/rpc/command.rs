// JS: dht-rpc/index.js
//! `enum DhtCommand`: messages from `DhtHandle` to the `DhtNode` actor.
//! Neighbours: `handle*` send them, `node_command` executes them.

use std::net::SocketAddr;

use tokio::sync::{mpsc, oneshot};

use libudx::UdxSocket;

use crate::messages::Ipv4Peer;
use crate::peer::NodeId;
use crate::query::QueryReply;

use super::requests::UserRequest;
use super::types::{DhtError, PingResponse, ResponseData, UserQueryParams, UserRequestParams};

// ── Internal command channel ──────────────────────────────────────────────────

pub(super) enum DhtCommand {
    Bootstrapped {
        reply_tx: oneshot::Sender<Result<(), DhtError>>,
    },
    Ping {
        host: String,
        port: u16,
        reply_tx: oneshot::Sender<Result<PingResponse, DhtError>>,
    },
    FindNode {
        target: NodeId,
        /// Session that owns the query (`None`: no owner).
        session: Option<u64>,
        reply_tx: oneshot::Sender<Result<Vec<QueryReply>, DhtError>>,
    },
    Query {
        params: UserQueryParams,
        session: Option<u64>,
        /// Listener of every successful reply with a value, as it arrives (`QueryStream`).
        values: Option<mpsc::UnboundedSender<QueryReply>>,
        reply_tx: oneshot::Sender<Result<Vec<QueryReply>, DhtError>>,
    },
    Request {
        params: UserRequestParams,
        host: String,
        port: u16,
        session: Option<u64>,
        /// External (held) socket to send from; `None` — the IO sockets.
        via: Option<UdxSocket>,
        reply_tx: oneshot::Sender<Result<ResponseData, DhtError>>,
    },
    /// Cancels everything owned by session `id` (requests, queries); unknown id is a no-op.
    DestroySession {
        id: u64,
    },
    Relay {
        command: u64,
        target: Option<NodeId>,
        value: Option<Vec<u8>>,
        to: Ipv4Peer,
        preserve_tid: Option<u16>,
        /// Send from this socket instead of the primary one (JS `opts.socket`).
        via: Option<UdxSocket>,
    },
    SendReplyTo {
        tid: u16,
        target: Option<NodeId>,
        to: Ipv4Peer,
        value: Option<Vec<u8>>,
    },
    SubscribeRequests {
        reply_tx: oneshot::Sender<mpsc::UnboundedReceiver<UserRequest>>,
    },
    TableSize {
        reply_tx: oneshot::Sender<usize>,
    },
    ToArray {
        limit: Option<usize>,
        reply_tx: oneshot::Sender<Vec<Ipv4Peer>>,
    },
    Destroy {
        reply_tx: oneshot::Sender<Result<(), DhtError>>,
    },
    Suspend {
        reply_tx: oneshot::Sender<()>,
    },
    Resume {
        reply_tx: oneshot::Sender<()>,
    },
    SetAdaptive {
        on: bool,
        reply_tx: oneshot::Sender<()>,
    },
    LocalPort {
        reply_tx: oneshot::Sender<Result<u16, DhtError>>,
    },
    TableId {
        reply_tx: oneshot::Sender<Option<NodeId>>,
    },
    ServerSocket {
        reply_tx: oneshot::Sender<Option<UdxSocket>>,
    },
    ListenSocket {
        reply_tx: oneshot::Sender<Option<UdxSocket>>,
    },
    RemoteAddress {
        reply_tx: oneshot::Sender<(Option<Ipv4Peer>, u64)>,
    },
    #[allow(dead_code)] // constructed by the held-socket forwarder task
    InboundReplyBytes {
        addr: SocketAddr,
        data: Vec<u8>,
    },
    /// A datagram received on a held socket `via`; replies to it leave from `via`.
    InboundVia {
        addr: SocketAddr,
        data: Vec<u8>,
        via: UdxSocket,
    },
    PingViaSocket {
        target: Ipv4Peer,
        socket: UdxSocket,
        reply_tx: oneshot::Sender<Result<PingResponse, DhtError>>,
    },
}
