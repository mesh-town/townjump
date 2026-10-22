// JS: dht-rpc/index.js
//! `UserRequest` (an inbound request handed to the user), `StandaloneRequest` and `DeferredReply`.
//! Neighbours: `node_responses` and `node_query` consume them; `command` carries them.

use tokio::sync::oneshot;

use libudx::UdxSocket;

use crate::io::ReplyContext;
use crate::messages::Ipv4Peer;
use crate::peer::NodeId;
use crate::routing_table::Node;

use super::consts::SUPPRESS_REPLY_SENTINEL;
use super::types::{DhtError, PingResponse, ResponseData};

/// An incoming user-facing request forwarded from the DHT.
#[non_exhaustive]
pub struct UserRequest {
    /// Origin peer for the request.
    pub from: Ipv4Peer,
    /// Optional origin peer id.
    pub id: Option<NodeId>,
    /// Transaction id from the inbound request, preserved end-to-end through
    /// any relay hops so the eventual REPLY matches the originator's inflight
    /// entry. Mirrors Node `dht-rpc/lib/io.js` Request.tid semantics.
    pub tid: u16,
    /// Optional request token.
    pub token: Option<[u8; 32]>,
    /// RPC command received.
    pub command: u64,
    /// Optional target node id.
    pub target: Option<NodeId>,
    /// Optional request payload.
    pub value: Option<Vec<u8>>,
    /// The socket the request arrived on (JS `req.socket`).
    pub arrived_on: UdxSocket,
    pub(super) reply_tx: Option<oneshot::Sender<(u64, Option<Vec<u8>>)>>,
}

impl UserRequest {
    /// Replies to the request with a value and success code.
    pub fn reply(&mut self, value: Option<Vec<u8>>) {
        if let Some(tx) = self.reply_tx.take() {
            let _ = tx.send((0, value));
        }
    }

    /// Replies to the request with an error code.
    pub fn error(&mut self, code: u64) {
        if let Some(tx) = self.reply_tx.take() {
            let _ = tx.send((code, None));
        }
    }

    /// Consume the request without emitting any reply. The deferred-reply
    /// pipeline that normally fires `ERR_UNKNOWN_COMMAND` on a dropped
    /// `reply_tx` is suppressed via an internal sentinel error code
    /// (`SUPPRESS_REPLY_SENTINEL`).
    ///
    /// Used by relay-forwarding paths that consume the inbound REQUEST
    /// without responding — the original requester's REPLY arrives via the
    /// tid-preserved relay chain, not via this hop. Mirrors Node behaviour:
    /// `dht-rpc::Request.relay(value, to)` does not also call
    /// `req.reply(...)`. Without this, the inbound `req` would auto-emit a
    /// REPLY using the preserved tid (= original client's tid), racing
    /// against the genuine REPLY arriving via the chain's tail and causing
    /// the client to match on the first arrival (typically an error).
    pub fn release(&mut self) {
        if let Some(tx) = self.reply_tx.take() {
            let _ = tx.send((SUPPRESS_REPLY_SENTINEL, None));
        }
    }
}

// ── Standalone (non-query) inflight tracking ──────────────────────────────────

pub(super) enum StandaloneRequest {
    Ping(oneshot::Sender<Result<PingResponse, DhtError>>),
    /// Ping sent from a puncher socket: answered like `Ping`, never a node NAT sample.
    PingViaSocket(oneshot::Sender<Result<PingResponse, DhtError>>),
    UserRequest(oneshot::Sender<Result<ResponseData, DhtError>>),
    /// One PING_NAT probe of the adaptive-mode check round; the round is implemented in `DhtNode::nat_check`.
    NatCheck,
    Reping {
        new_node: Node,
        old_node_id: NodeId,
        last_seen_tick: u64,
    },
    Check {
        node_id: NodeId,
        last_seen_tick: u64,
    },
}

pub(super) struct DeferredReply {
    pub(super) from: Ipv4Peer,
    pub(super) reply_ctx: ReplyContext,
    pub(super) tid: u16,
    pub(super) target: Option<NodeId>,
    pub(super) error: u64,
    pub(super) value: Option<Vec<u8>>,
}
