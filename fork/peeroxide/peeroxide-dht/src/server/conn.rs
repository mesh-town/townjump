// JS: hyperdht/lib/server.js
//! A connection in the making: the UDX stream and Noise session of one
//! handshake, and the two ways to finish it (`dial` and `finish`).

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use libudx::{UdxAsyncStream, UdxSocket, UdxStream};
use tokio::sync::mpsc::{self, UnboundedSender};
use tokio::sync::oneshot;
use tokio::time::timeout;

use super::actor::Internal;
use super::punch::PunchState;
use super::relay::RelayAttempt;
use crate::hyperdht::{HyperDhtError, PeerConnection, Reuse};
use crate::hyperdht_messages::NoisePayload;
use crate::messages::Ipv4Peer;
use crate::noise_wrap::NoiseWrapResult;
use crate::secret_stream::SecretStream;
use crate::socket_pool::UdxFrameSeen;

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

/// True while the puzzle task still holds its stop receiver.
fn probing(stop: Option<&oneshot::Sender<()>>) -> bool {
    stop.is_some_and(|s| !s.is_closed())
}

impl Pending {
    /// True while the puzzle of this slot is still running.
    pub(super) fn puzzle_alive(&self) -> bool {
        probing(self.punch.as_ref().and_then(|p| p.stop.as_ref()))
    }

    /// The slot timed out: like [`Pending::destroy`], and says so if the puzzle still probes.
    pub(super) fn expire(self) {
        if self.puzzle_alive() {
            tracing::debug!(
                remote_udx_id = self.remote_udx_id,
                "server: handshake slot expired while the puzzle still probes"
            );
        }
        self.destroy();
    }

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
        remote_udx_id,
        punch,
        keep,
        ..
    } = p;
    // Decided before the puzzle is destroyed: only a punch socket may be held.
    let punched = punch.is_some();
    // The client may have reached a birthday socket of the puzzle instead of the primary one.
    let moved = punch.as_ref().and_then(PunchState::moved_socket);
    let socket = adopt_moved_socket(&stream, socket, moved, remote_udx_id, addr).await;
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

/// Where the puzzle reports a UDX frame that reached one of its sockets, and the socket it came on.
#[derive(Clone)]
pub(super) struct FollowStream {
    id: u64,
    internal: UnboundedSender<Internal>,
    moved: Arc<Mutex<Option<UdxSocket>>>,
}

impl FollowStream {
    pub(super) fn new(id: u64, internal: UnboundedSender<Internal>) -> Self {
        Self {
            id,
            internal,
            moved: Arc::new(Mutex::new(None)),
        }
    }

    /// The socket the first frame came on, if one has come.
    pub(super) fn moved_socket(&self) -> Option<UdxSocket> {
        let cell = self.moved.lock().unwrap_or_else(|e| e.into_inner());
        cell.clone()
    }
}

/// Waits for the first UDX frame on a puzzle socket: stores its socket, then tells the actor the
/// client arrived. Returns after that frame (dropping `frames` closes the sinks of the pool); no
/// frame, no event.
pub(super) async fn follow_frames(
    f: FollowStream,
    mut frames: mpsc::UnboundedReceiver<UdxFrameSeen>,
) {
    let Some(seen) = frames.recv().await else {
        return;
    };
    let mut cell = f.moved.lock().unwrap_or_else(|e| e.into_inner());
    *cell = Some(seen.socket);
    drop(cell);
    let _ = f.internal.send(Internal::Arrived {
        id: f.id,
        addr: seen.addr,
    });
}

/// Runs the stream on the socket that got the client's first frame. Without one, or when the
/// stream cannot be moved (its hook already fired on the original socket), the original stays.
pub(super) async fn adopt_moved_socket(
    stream: &UdxStream,
    socket: UdxSocket,
    moved: Option<UdxSocket>,
    remote_id: u32,
    addr: SocketAddr,
) -> UdxSocket {
    let Some(other) = moved else {
        return socket;
    };
    match stream.connect(&other, remote_id, addr).await {
        Ok(()) => other,
        Err(e) => {
            tracing::debug!(err = %e, %addr, "server: stream stays on its socket");
            socket
        }
    }
}

#[cfg(test)]
#[path = "conn_tests.rs"]
mod tests;
