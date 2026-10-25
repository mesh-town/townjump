// JS: dht-rpc/lib/io.js
//! Public and internal types of the IO layer: errors, config, stats, events, in-flight entries.
//! Neighbours: `window` (congestion), `state` (`Io` itself).

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use portable_atomic::AtomicU64;
use std::time::Duration;

use blake2::Blake2bMac;
use blake2::digest::consts::U32;
use libudx::UdxSocket;
use tokio::time::Instant;

use crate::messages::Ipv4Peer;
use crate::peer::NodeId;

pub(super) type Blake2bMac256 = Blake2bMac<U32>;

// ── Errors ────────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum IoError {
    #[error("UDP error: {0}")]
    Udx(#[from] libudx::UdxError),
    #[error("address parse error: {0}")]
    AddrParse(#[from] std::net::AddrParseError),
    #[error("encoding error: {0}")]
    Encoding(#[from] crate::compact_encoding::EncodingError),
    #[error("routing table lock poisoned")]
    LockPoisoned,
}

pub type IoResult<T> = Result<T, IoError>;

// ── Public config / stats / types ─────────────────────────────────────────────

/// IO layer configuration.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct IoConfig {
    pub max_window: usize,
    pub port: u16,
    pub host: String,
    pub firewalled: bool,
    pub ephemeral: bool,
}

impl Default for IoConfig {
    fn default() -> Self {
        Self {
            max_window: 80,
            port: 0,
            host: "0.0.0.0".to_string(),
            firewalled: true,
            ephemeral: true,
        }
    }
}

/// IO layer statistics.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct IoStats {
    pub active: u64,
    pub total: u64,
    pub responses: u64,
    pub timeouts: u64,
    pub retries: u64,
    /// Requests dropped by [`Io::cancel_owner`] (owner gone) before a reply or final timeout.
    pub cancelled: u64,
}

/// Wire-byte counters shared between the IO layer and consumers (e.g. progress
/// reporters in a command-line tool). Increments are `Relaxed` — these are
/// observability metrics, not synchronization primitives.
///
/// The counters track every UDP datagram the IO layer hands to or receives
/// from the OS sockets, regardless of which protocol layer originated it
/// (queries, requests, replies, relays, retries — all counted).
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct WireCounters {
    pub bytes_sent: Arc<AtomicU64>,
    pub bytes_received: Arc<AtomicU64>,
}

impl WireCounters {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a `WireCounters` snapshot from externally-shared atomic counters.
    ///
    /// Use when a downstream consumer (e.g. a UI progress reporter) already
    /// owns `Arc<AtomicU64>` byte counters and wants a `WireCounters` view
    /// over them. The returned value shares the atomics with the caller.
    pub fn from_counters(bytes_sent: Arc<AtomicU64>, bytes_received: Arc<AtomicU64>) -> Self {
        Self {
            bytes_sent,
            bytes_received,
        }
    }

    pub fn snapshot(&self) -> (u64, u64) {
        (
            self.bytes_sent.load(Ordering::Relaxed),
            self.bytes_received.load(Ordering::Relaxed),
        )
    }
}

/// Which socket was used for a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketKind {
    Client,
    Server,
}

/// Info about an inflight request that was resolved by a response.
#[derive(Debug, Clone)]
pub struct ResolvedRequest {
    pub tid: u16,
    pub to: Ipv4Peer,
    pub command: u64,
    pub internal: bool,
    pub target: Option<NodeId>,
}

/// Events emitted by the IO layer.
#[non_exhaustive]
pub enum IoEvent {
    IncomingRequest(IncomingRequest),
    Response {
        tid: u16,
        from: Ipv4Peer,
        to: Ipv4Peer,
        id: Option<NodeId>,
        token: Option<[u8; 32]>,
        closer_nodes: Vec<Ipv4Peer>,
        error: u64,
        value: Option<Vec<u8>>,
        rtt: Duration,
        request: ResolvedRequest,
        /// Arrived on the server socket directly (not the client socket, not an external one).
        server_arrival: bool,
    },
}

/// An incoming request (server-side).
pub struct IncomingRequest {
    pub tid: u16,
    pub from: Ipv4Peer,
    pub to: Ipv4Peer,
    pub id: Option<NodeId>,
    pub token: Option<[u8; 32]>,
    pub internal: bool,
    pub command: u64,
    pub target: Option<NodeId>,
    pub value: Option<Vec<u8>>,
    pub(crate) reply_ctx: ReplyContext,
}

/// Parameters for creating an outgoing request.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct RequestParams {
    pub to: Ipv4Peer,
    pub token: Option<[u8; 32]>,
    pub internal: bool,
    pub command: u64,
    pub target: Option<NodeId>,
    pub value: Option<Vec<u8>>,
    pub timeout_ms: Option<u64>,
    pub retries: Option<u32>,
    /// Destination node id; `Some` means the node is dropped from the table on the final timeout.
    pub node_id: Option<NodeId>,
}

/// A timeout event — emitted when a request exceeds all retries.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct TimeoutEvent {
    pub tid: u16,
    pub to: Ipv4Peer,
    pub command: u64,
    pub internal: bool,
    pub target: Option<NodeId>,
    /// Destination node id; `Some` means the node is dropped from the table on the final timeout.
    pub node_id: Option<NodeId>,
}

/// Intermediate expiry of the request timer: the retransmission is already sent.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub(crate) struct RetryEvent {
    /// Request transaction identifier.
    pub tid: u16,
    /// Request destination.
    pub to: Ipv4Peer,
}

// ── Private types ─────────────────────────────────────────────────────────────

/// Where an incoming request arrived: replies go back the same way. No `Debug`/`Copy`: `UdxSocket` has neither.
#[derive(Clone)]
pub(crate) struct ReplyContext {
    pub(crate) socket_kind: SocketKind,
    /// External (held) socket the request came through; `None` — one of the IO sockets.
    pub(crate) via: Option<UdxSocket>,
}

/// Bundled parameters for `send_reply_internal` to stay under clippy's argument limit.
pub(super) struct ReplyInternalParams {
    pub(super) socket_kind: SocketKind,
    pub(super) tid: u16,
    pub(super) target: Option<NodeId>,
    pub(super) error: u64,
    pub(super) include_token: bool,
    pub(super) value: Option<Vec<u8>>,
    /// Answer out of this external socket (no id), instead of the IO socket of `socket_kind`.
    pub(super) via: Option<UdxSocket>,
}

/// Who a request belongs to: cancelling the owner drops its in-flight requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Owner {
    /// Request of the query with this id.
    Query(u64),
    /// Request made through the session with this id.
    Session(u64),
}

pub(super) struct InflightEntry {
    pub(super) tid: u16,
    pub(super) to: Ipv4Peer,
    pub(super) addr: SocketAddr,
    pub(super) internal: bool,
    pub(super) command: u64,
    pub(super) target: Option<NodeId>,
    pub(super) node_id: Option<NodeId>,
    pub(super) buffer: Vec<u8>,
    pub(super) socket_kind: SocketKind,
    pub(super) sent: u32,
    pub(super) retries: u32,
    pub(super) deadline: Instant,
    pub(super) timestamp: Instant,
    pub(super) timeout: Duration,
    pub(super) owner: Option<Owner>,
    /// External socket for every send of this entry (first, queued, retries); `None` — IO sockets.
    pub(super) via: Option<UdxSocket>,
}

pub(super) struct PendingSend {
    pub(super) tid: u16,
}
