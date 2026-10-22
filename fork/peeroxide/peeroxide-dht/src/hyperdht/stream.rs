//! Stream ids and turning a finished handshake into a `SecretStream` over UDX.
//! Neighbours: `connect_finish`, `connect_relayed` and `server_run` call `establish_stream*`.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, Ordering};

use libudx::{UdxRuntime, UdxSocket, UdxStream};

use crate::secret_stream::SecretStream;

use super::connection::{ConnectResult, PeerConnection};
#[cfg(doc)]
use super::handle::HyperDhtHandle;
use super::types::HyperDhtError;

// ── Errors ────────────────────────────────────────────────────────────────────

static NEXT_STREAM_ID: AtomicU32 = AtomicU32::new(1);

pub(super) fn next_stream_id() -> u32 {
    NEXT_STREAM_ID.fetch_add(1, Ordering::Relaxed)
}

/// Allocate a fresh, process-wide-unique local UDX stream id.
///
/// Exposed so callers outside this crate that reuse a `HyperDhtHandle`'s
/// connections (e.g. upstream `peeroxide::swarm`'s relay-through server path) can
/// mint additional stream ids from the *same* counter this module uses
/// internally for control connections — using a separately-counted id
/// (e.g. a crate-local counter starting at 1 again) risks colliding with
/// an id this crate already registered on a shared, reused socket.
pub fn alloc_stream_id() -> u32 {
    next_stream_id()
}

/// Create a UDX stream, connect it to the remote peer, and wrap with
/// [`SecretStream::from_session`] using the Noise handshake keys.
///
/// Call after [`HyperDhtHandle::connect`] to upgrade a [`ConnectResult`]
/// into an encrypted bidirectional stream.
///
/// A fresh UDX socket bound to an ephemeral port is created for the stream.
/// To reuse an existing socket (Node.js-style single-socket multiplexing),
/// use [`establish_stream_with_socket`] instead.
pub async fn establish_stream(
    result: &ConnectResult,
    runtime: &UdxRuntime,
) -> Result<PeerConnection, HyperDhtError> {
    establish_stream_with_socket(result, runtime, None).await
}

/// Call after [`HyperDhtHandle::connect`] to upgrade a [`ConnectResult`]
/// into an encrypted bidirectional stream, optionally reusing an existing socket.
///
/// When `shared_socket` is `Some`, the stream reuses that socket (matching
/// the Node.js single-socket multiplexing model). When `None`, a fresh socket
/// bound to an ephemeral port is created.
pub async fn establish_stream_with_socket(
    result: &ConnectResult,
    runtime: &UdxRuntime,
    shared_socket: Option<UdxSocket>,
) -> Result<PeerConnection, HyperDhtError> {
    udx_target(result)?; // rejected before the socket is created
    let socket = if let Some(s) = shared_socket {
        s
    } else {
        let s = runtime.create_socket().await?;
        s.bind("0.0.0.0:0".parse().expect("valid addr")).await?;
        s
    };
    let stream = runtime.create_stream(result.local_stream_id).await?;
    establish_on_stream(result, stream, socket).await
}

/// Remote UDX stream id and server address of a finished handshake.
fn udx_target(result: &ConnectResult) -> Result<(u32, SocketAddr), HyperDhtError> {
    let remote_udx = result
        .remote_udx
        .as_ref()
        .ok_or_else(|| HyperDhtError::StreamEstablishment("no remote UDX info".into()))?;

    let remote_id = u32::try_from(remote_udx.id)
        .map_err(|_| HyperDhtError::StreamEstablishment("remote UDX id out of u32 range".into()))?;

    let addr: SocketAddr = SocketAddr::new(
        result
            .server_address
            .host
            .parse()
            .map_err(|e| HyperDhtError::StreamEstablishment(format!("invalid address: {e}")))?,
        result.server_address.port,
    );
    Ok((remote_id, addr))
}

/// Connect an already created `stream` (possibly registered on `socket` earlier) and wrap it with
/// the Noise keys of `result`.
pub(super) async fn establish_on_stream(
    result: &ConnectResult,
    stream: UdxStream,
    socket: UdxSocket,
) -> Result<PeerConnection, HyperDhtError> {
    let (remote_id, addr) = udx_target(result)?;
    tracing::debug!(
        target: "peeroxide_dht::hyperdht::connect",
        local_id = stream.local_id(),
        remote_id,
        %addr,
        "establishing UDX stream"
    );
    stream.connect(&socket, remote_id, addr).await?;

    let async_stream = stream.into_async_stream();
    let ss = SecretStream::from_session(
        result.noise.is_initiator,
        async_stream,
        result.noise.tx,
        result.noise.rx,
        result.noise.handshake_hash,
        result.noise.remote_public_key,
    )
    .await?;
    tracing::debug!(
        target: "peeroxide_dht::hyperdht::connect",
        "SecretStream established"
    );

    Ok(PeerConnection {
        remote_public_key: result.remote_public_key,
        stream: ss,
        remote_addr: Some(addr),
        socket,
        _relay_task: None,
        relay: None,
        lease: None,
    })
}
