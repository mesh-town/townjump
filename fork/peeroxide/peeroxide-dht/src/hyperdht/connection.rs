//! `ConnectResult` and `PeerConnection`: what a finished connect hands to the caller.
//! Neighbours: `connect_finish` and `connect_relayed` build them; `stream` fills `PeerConnection`.

use std::fmt;

use tokio::sync::watch;
use tokio::task::JoinHandle;

use libudx::{UdxAsyncStream, UdxSocket};

use crate::hyperdht_messages::UdxInfo;
use crate::messages::Ipv4Peer;
use crate::noise_wrap::NoiseWrapResult;
use crate::secret_stream::SecretStream;

use super::relay::{RelayLink, RelayPath};
use super::reuse::Lease;

#[derive(Clone)]
/// Metadata needed to establish a peer connection.
#[non_exhaustive]
pub struct ConnectResult {
    /// Remote peer's public key.
    pub remote_public_key: [u8; 32],
    /// Address used to reach the server during handshake.
    pub server_address: Ipv4Peer,
    /// Address of the client-side peer endpoint.
    pub client_address: Ipv4Peer,
    /// Whether the connection was relayed through a third party.
    pub is_relayed: bool,
    /// Final Noise state and negotiated keys.
    pub noise: NoiseWrapResult,
    /// Local UDX stream id to use for the connection.
    pub local_stream_id: u32,
    /// Remote UDX metadata advertised by the peer.
    pub remote_udx: Option<UdxInfo>,
    /// Clone of the puncher's primary UDX socket when holepunching lands.
    ///
    /// When set by `run_holepunch_rounds`' `Connected` arm, this is the
    /// socket whose punched 4-tuple is established.
    /// Consumers should prefer it over the DHT server socket when present.
    pub udx_socket: Option<UdxSocket>,
}

impl fmt::Debug for ConnectResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectResult")
            .field("remote_public_key", &self.remote_public_key)
            .field("server_address", &self.server_address)
            .field("client_address", &self.client_address)
            .field("is_relayed", &self.is_relayed)
            .field("noise", &self.noise)
            .field("local_stream_id", &self.local_stream_id)
            .field("remote_udx", &self.remote_udx)
            .field(
                "udx_socket",
                &self.udx_socket.as_ref().map(|_| "Some(UdxSocket)"),
            )
            .finish()
    }
}

/// Established encrypted connection to a peer.
///
/// Wraps a [`SecretStream`] over a UDX transport, keeping the underlying
/// socket alive for the connection's lifetime.
#[non_exhaustive]
pub struct PeerConnection {
    /// Encrypted bidirectional stream to the peer.
    pub stream: SecretStream<UdxAsyncStream>,
    /// Remote peer's public key.
    pub remote_public_key: [u8; 32],
    /// Remote peer's network address (used by server-side relay to connect data streams).
    pub remote_addr: Option<std::net::SocketAddr>,
    /// The UDX socket underlying this connection. Public so relay flows
    /// in downstream crates can reuse the control channel's socket for
    /// data streams (matching Node.js behaviour).
    pub socket: UdxSocket,
    pub(super) _relay_task: Option<JoinHandle<()>>,
    /// The relay this connection runs over (dropped after the stream, before the lease).
    pub(super) relay: Option<RelayLink>,
    /// Keeps a held punch socket open while the connection is alive (declared last: dropped after the stream).
    pub(super) lease: Option<Lease>,
}

impl PeerConnection {
    /// Create a new peer connection from its components.
    pub fn new(
        stream: SecretStream<UdxAsyncStream>,
        remote_public_key: [u8; 32],
        socket: UdxSocket,
        relay_task: Option<JoinHandle<()>>,
    ) -> Self {
        Self {
            stream,
            remote_public_key,
            remote_addr: None,
            socket,
            _relay_task: relay_task,
            relay: None,
            lease: None,
        }
    }

    /// Create a new peer connection with a known remote address.
    pub fn with_remote_addr(
        stream: SecretStream<UdxAsyncStream>,
        remote_public_key: [u8; 32],
        remote_addr: std::net::SocketAddr,
        socket: UdxSocket,
        relay_task: Option<JoinHandle<()>>,
    ) -> Self {
        Self {
            stream,
            remote_public_key,
            remote_addr: Some(remote_addr),
            socket,
            _relay_task: relay_task,
            relay: None,
            lease: None,
        }
    }

    /// The current path of a relayed connection; `None` when it never used a relay.
    pub fn relay_path(&self) -> Option<watch::Receiver<RelayPath>> {
        self.relay.as_ref().map(RelayLink::path)
    }

    /// Attach the relay link of a connection that runs over a relay.
    pub(crate) fn set_relay(&mut self, link: RelayLink) {
        self.relay = Some(link);
    }

    /// Tie a socket lease to this connection; a previous lease, if any, is released.
    pub(crate) fn hold(&mut self, lease: Lease) {
        self.lease = Some(lease);
    }
}

impl fmt::Debug for PeerConnection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PeerConnection")
            .field("remote_public_key", &&self.remote_public_key[..8])
            .field("remote_addr", &self.remote_addr)
            .field(
                "relayed",
                &(self._relay_task.is_some() || self.relay.is_some()),
            )
            .finish_non_exhaustive()
    }
}
