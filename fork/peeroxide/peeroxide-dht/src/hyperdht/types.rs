//! Errors, server events and the result records of lookup/announce/put/get.
//! Neighbours: `handle` and the API files (`lookup`, `announce`, `records`) return them.

use thiserror::Error;
use tokio::sync::oneshot;

use crate::blind_relay::RelayError;
use crate::hyperdht_messages::{HandshakeMessage, HolepunchMessage, HyperPeer, RelayInfo};
use crate::messages::Ipv4Peer;
use crate::peer::NodeId;
use crate::rpc::DhtError;
use crate::secret_stream::SecretStreamError;

#[derive(Debug, Error)]
/// Errors returned by HyperDHT operations.
#[non_exhaustive]
pub enum HyperDhtError {
    /// Error propagated from the underlying DHT client.
    #[error("DHT error: {0}")]
    Dht(#[from] DhtError),
    /// Error while encoding or decoding protocol data.
    #[error("encoding error: {0}")]
    Encoding(#[from] crate::compact_encoding::EncodingError),
    /// Error from Noise handshake or session setup.
    #[error("noise error: {0}")]
    Noise(#[from] crate::noise::NoiseError),
    /// Error from the Noise wrapper layer.
    #[error("noise wrap error: {0}")]
    NoiseWrap(#[from] crate::noise_wrap::NoiseWrapError),
    /// Error from the router state machine.
    #[error("router error: {0}")]
    Router(#[from] crate::router::RouterError),
    /// Error while wrapping or unwrapping secure payloads.
    #[error("secure payload error: {0}")]
    SecurePayload(#[from] crate::secure_payload::SecurePayloadError),
    /// This DHT instance has been destroyed.
    #[error("node destroyed")]
    Destroyed,
    /// A signature did not verify.
    #[error("invalid signature")]
    InvalidSignature,
    /// A content hash did not match.
    #[error("invalid hash")]
    InvalidHash,
    /// The internal channel was closed.
    #[error("channel closed")]
    ChannelClosed,
    /// No peer was found for the requested target.
    #[error("peer not found")]
    PeerNotFound,
    /// No relay nodes were available for the operation.
    #[error("no relay nodes available")]
    NoRelayNodes,
    /// The handshake failed with the given message.
    #[error("handshake failed: {0}")]
    HandshakeFailed(String),
    /// Hole punching did not succeed.
    #[error("holepunch failed")]
    HolepunchFailed,
    /// Hole punching was aborted by the remote side.
    #[error("holepunch aborted")]
    HolepunchAborted,
    /// The remote firewall rejected the connection.
    #[error("firewall rejected")]
    FirewallRejected,
    /// Error from the UDX transport layer.
    #[error("UDX error: {0}")]
    Udx(#[from] libudx::UdxError),
    /// Error from the secret stream layer.
    #[error("secret stream error: {0}")]
    SecretStream(#[from] SecretStreamError),
    /// Failed to establish a UDX stream.
    #[error("stream establishment failed: {0}")]
    StreamEstablishment(String),
    /// Error from the relay subsystem.
    #[error("relay error: {0}")]
    Relay(#[from] RelayError),
    /// The server is already listening (or starting to listen).
    #[error("server is already listening")]
    AlreadyListening,
    /// The server has been closed and cannot be used again.
    #[error("server closed")]
    ServerClosed,
    /// The blind-relay pairing did not complete (timeout or any failure on the way).
    #[error("relay aborted")]
    RelayAborted,
}

// ── Server events (forwarded to listen() subscribers) ────────────────────────

#[derive(Debug)]
/// Events forwarded to server-side listeners.
#[non_exhaustive]
pub enum ServerEvent {
    /// A peer handshake request that may need local server handling.
    PeerHandshake {
        /// The decoded handshake message.
        msg: HandshakeMessage,
        /// Address of the peer that sent the request to us.
        ///
        /// For a direct (non-forwarded) handshake, this is the remote peer.
        /// For a relayed handshake, this is the forwarding DHT node (the
        /// previous hop), NOT the originating peer; use `peer_address` for
        /// the originating peer's address in that case.
        from: Ipv4Peer,
        /// Address of the originating peer for a forwarded handshake, or
        /// `None` for a direct handshake.
        ///
        /// Populated from the relay-layer's `peer_address` payload field.
        /// Mirrors Node Hyperswarm's "peerAddress" semantics in
        /// `lib/router.js`. Servers should prefer this over `from` when
        /// choosing a remote address to dial.
        peer_address: Option<Ipv4Peer>,
        /// Optional DHT target associated with the request.
        target: Option<NodeId>,
        /// Reply channel for the generated response.
        reply_tx: oneshot::Sender<Option<Vec<u8>>>,
        /// Socket the request arrived on and the socket to answer from.
        route: crate::handshake_route::HandshakeRoute,
    },
    /// A peer holepunch request that may need local server handling.
    PeerHolepunch {
        /// The decoded holepunch message.
        msg: HolepunchMessage,
        /// Address of the peer that sent the request.
        from: Ipv4Peer,
        /// Address of the peer we should punch toward.
        peer_address: Ipv4Peer,
        /// Optional DHT target associated with the request.
        target: Option<NodeId>,
        /// Reply channel for the generated response.
        reply_tx: oneshot::Sender<Option<Vec<u8>>>,
    },
}

// ── Result types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
/// Result from a LOOKUP query.
#[non_exhaustive]
pub struct LookupResult {
    /// Node that returned the lookup result.
    pub from: Ipv4Peer,
    /// Optional intermediate hop used to reach the node.
    pub to: Option<Ipv4Peer>,
    /// Peers advertised by the node.
    pub peers: Vec<HyperPeer>,
}

#[derive(Debug, Clone)]
/// Result from an ANNOUNCE operation.
#[non_exhaustive]
pub struct AnnounceResult {
    /// Closest nodes contacted during the announce.
    pub closest_nodes: Vec<Ipv4Peer>,
    /// `{relay_address: from, peer_address: to}` of every node that was sent an ANNOUNCE
    /// (`announce`, no cap), or of the at most 3 that accepted it (a server's announce, as the
    /// JS announcer): `peer_address` is our address as seen by that node.
    pub relays: Vec<RelayInfo>,
}

#[derive(Debug, Clone)]
/// Result from an immutable put operation.
#[non_exhaustive]
pub struct ImmutablePutResult {
    /// Content hash used as the target key.
    pub hash: [u8; 32],
    /// Closest nodes contacted during the write.
    pub closest_nodes: Vec<Ipv4Peer>,
}

#[derive(Debug, Clone)]
/// Result from a mutable put operation.
#[non_exhaustive]
pub struct MutablePutResult {
    /// Public key used as the mutable record key.
    pub public_key: [u8; 32],
    /// Closest nodes contacted during the write.
    pub closest_nodes: Vec<Ipv4Peer>,
    /// Record sequence number that was written.
    pub seq: u64,
    /// Signature over the stored value.
    pub signature: [u8; 64],
    /// Number of commit-phase requests that timed out.
    pub commit_timeouts: u32,
}

#[derive(Debug, Clone)]
/// Result from a mutable get operation.
#[non_exhaustive]
pub struct MutableGetResult {
    /// Retrieved value bytes.
    pub value: Vec<u8>,
    /// Sequence number attached to the value.
    pub seq: u64,
    /// Signature verifying the value.
    pub signature: [u8; 64],
    /// Node that returned the value.
    pub from: Ipv4Peer,
}
