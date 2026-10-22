// JS: hyperdht/lib/server.js
//! A connection in the making: the UDX stream and Noise session of one
//! handshake, and the two ways to finish it (`dial` and `finish`).

use std::net::SocketAddr;
use std::time::Duration;

use libudx::{UdxAsyncStream, UdxSocket, UdxStream};
use tokio::sync::mpsc::{self, UnboundedSender};
use tokio::time::timeout;

use super::actor::Internal;
use super::punch::PunchState;
use super::relay::RelayAttempt;
use crate::hyperdht::{HyperDhtError, PeerConnection, Reuse};
use crate::hyperdht_messages::NoisePayload;
use crate::messages::Ipv4Peer;
use crate::noise_wrap::NoiseWrapResult;
use crate::secret_stream::SecretStream;

/// How long the SecretStream identity exchange may take.
const SESSION_TIMEOUT: Duration = Duration::from_secs(10);

/// Everything of one handshake that is needed to turn it into a `PeerConnection`.
pub(super) struct Pending {
    /// Our UDX stream (created, not yet connected).
    pub(super) stream: UdxStream,
    /// The socket the stream runs on (its own clone; the pool may be gone).
    pub(super) socket: UdxSocket,
    /// The final Noise session (keys, handshake hash, client key).
    pub(super) noise: NoiseWrapResult,
    /// The client's UDX stream id.
    pub(super) remote_udx_id: u32,
    /// The puzzle, when both sides are behind NAT.
    pub(super) punch: Option<PunchState>,
    /// Set when both sides agreed on reusable sockets: keep the socket after the connect.
    pub(super) keep: Option<Reuse>,
    /// The relay pairing in progress, when the handshake named a relay.
    pub(super) relay: Option<RelayAttempt>,
}

impl Pending {
    /// Gives the connection up: destroys the puzzle, drops the stream.
    pub(super) fn destroy(self) {
        if let Some(punch) = self.punch {
            punch.destroy();
        }
    }
}

/// `Ipv4Peer` (host text and port) as a socket address; `None` if the host is not an IP.
pub(super) fn to_socket_addr(peer: &Ipv4Peer) -> Option<SocketAddr> {
    format!("{}:{}", peer.host, peer.port).parse().ok()
}

/// Sends first: connects the stream to `addr`, then finishes the connection.
pub(super) async fn dial(p: Pending, addr: SocketAddr, tx: mpsc::Sender<PeerConnection>) {
    if let Err(e) = p.stream.connect(&p.socket, p.remote_udx_id, addr).await {
        tracing::debug!(err = %e, %addr, "server: dial failed");
        p.destroy();
        return;
    }
    finish(p, addr, tx).await;
}

/// Builds the encrypted stream over the (connected) UDX stream and hands the
/// connection to the incoming channel; a full or closed channel drops it.
pub(super) async fn finish(p: Pending, addr: SocketAddr, tx: mpsc::Sender<PeerConnection>) {
    let Pending {
        stream,
        socket,
        noise,
        punch,
        keep,
        ..
    } = p;
    // Decided before the puzzle is destroyed: only a punch socket may be held.
    let punched = punch.is_some();
    if let Some(punch) = punch {
        punch.destroy();
    }
    let Some(session) = session(stream, &noise).await else {
        return;
    };
    let key = noise.remote_public_key;
    let lease = keep.and_then(|k| {
        if punched {
            k.hold(&socket)
        } else {
            k.lease(&socket)
        }
    });
    tracing::debug!(%addr, punched, kept = lease.is_some(), "server: connection socket");
    let mut conn = PeerConnection::with_remote_addr(session, key, addr, socket, None);
    if let Some(lease) = lease {
        conn.hold(lease);
    }
    if tx.try_send(conn).is_err() {
        tracing::debug!(%addr, "server: incoming channel is full or closed, connection dropped");
    }
}

pub(super) async fn session(
    stream: UdxStream,
    noise: &NoiseWrapResult,
) -> Option<SecretStream<UdxAsyncStream>> {
    let raw = stream.into_async_stream();
    let (tx, rx, hash, key) = (
        noise.tx,
        noise.rx,
        noise.handshake_hash,
        noise.remote_public_key,
    );
    let opened = SecretStream::from_session(false, raw, tx, rx, hash, key);
    match timeout(SESSION_TIMEOUT, opened).await {
        Ok(Ok(session)) => Some(session),
        Ok(Err(e)) => {
            tracing::debug!(err = %e, "server: secret stream failed");
            None
        }
        Err(_) => {
            tracing::debug!("server: secret stream timed out");
            None
        }
    }
}

/// The client's UDX stream id from its Noise payload.
pub(super) fn remote_udx_id(remote: &NoisePayload) -> Result<u32, HyperDhtError> {
    let udx = remote
        .udx
        .as_ref()
        .ok_or_else(|| HyperDhtError::HandshakeFailed("client sent no UDX info".into()))?;
    u32::try_from(udx.id)
        .map_err(|_| HyperDhtError::StreamEstablishment("remote UDX id out of u32 range".into()))
}

/// The client's first packet commits its address to the stream; the actor is told.
pub(super) fn arm_hook(
    stream: &UdxStream,
    socket: &UdxSocket,
    remote_id: u32,
    id: u64,
    internal: &UnboundedSender<Internal>,
) -> Result<(), HyperDhtError> {
    let tx = internal.clone();
    stream.set_firewall_hook(socket, remote_id, move |_, port, ip| {
        let addr = SocketAddr::new(ip, port);
        let _ = tx.send(Internal::Arrived { id, addr });
        true
    })?;
    Ok(())
}
