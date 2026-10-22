//! Client retry over the route of an earlier connection (`retry_route`) and the finish of such a
//! retry (`finish_via`). Neighbours: `connect` (entry point), `connect_finish` (cold path), `reuse`.
// JS: hyperdht/lib/connect.js:171-190, 543-545

use libudx::{UdxRuntime, UdxSocket, UdxStream};

use super::config::ConnectOpts;
use super::connect_flow::Handshaken;
use super::connection::{ConnectResult, PeerConnection};
use super::handle::HyperDhtHandle;
use super::keypair::KeyPair;
use super::stream::{establish_on_stream, next_stream_id};
use super::types::HyperDhtError;

/// Socket of an earlier connection and the client stream already registered on it: the server's
/// first packet can arrive before its reply (JS `udx.c:2366`, the stream is in the socket's table
/// from creation). Dropping the value (any exit without a connection) unregisters the stream.
pub(super) struct Via {
    pub(super) socket: UdxSocket,
    pub(super) stream: UdxStream,
}

impl Via {
    pub(super) async fn open(
        runtime: &UdxRuntime,
        socket: &UdxSocket,
    ) -> Result<Via, HyperDhtError> {
        let stream = runtime.create_stream(next_stream_id()).await?;
        stream.register(socket)?;
        Ok(Via {
            socket: socket.clone(),
            stream,
        })
    }
}

impl HyperDhtHandle {
    /// PEER_HANDSHAKE straight to the server over the socket of an earlier connection; `None`
    /// (no route, any failure) sends the caller on to FIND_PEER. A failed route is dropped.
    pub(super) async fn retry_route(
        &self,
        key_pair: &KeyPair,
        remote_public_key: &[u8; 32],
        runtime: &UdxRuntime,
        opts: &ConnectOpts,
    ) -> Option<PeerConnection> {
        // `route.lease` keeps the socket open until this function returns (JS `ref.active()`).
        let route = self.reuse.route(remote_public_key)?;
        let _lease = route.lease;
        let via = match Via::open(runtime, &route.socket).await {
            Ok(via) => via,
            Err(e) => return self.route_failed(remote_public_key, &route.socket, &e.to_string()),
        };
        let session = self.dht.session();
        let me = self.in_session(&session);
        let handshaken = me
            .handshake_through_node(key_pair, remote_public_key, &route.addr, Some(via), opts)
            .await;
        let handshaken = match handshaken {
            Ok(h) if !h.hs_result.relayed => h,
            Ok(_) => return self.route_failed(remote_public_key, &route.socket, "relayed reply"),
            Err(e) => return self.route_failed(remote_public_key, &route.socket, &e.to_string()),
        };
        // JS `connect.js:547`: the session's other requests stop before the stream is set up.
        session.destroy();
        match me
            .finish_connection(handshaken, key_pair, remote_public_key, runtime, opts)
            .await
        {
            Ok(conn) => Some(conn),
            Err(e) => self.route_failed(remote_public_key, &route.socket, &e.to_string()),
        }
    }

    fn route_failed(
        &self,
        key: &[u8; 32],
        socket: &UdxSocket,
        why: &str,
    ) -> Option<PeerConnection> {
        tracing::debug!(
            why,
            "retry_route: failed, route dropped, falling back to FIND_PEER"
        );
        self.reuse.drop_route(key, socket);
        None
    }

    /// `finish_connection` for a handshake that went over `via`: no holepunch, the stream goes
    /// to the route address on the same socket (JS `connect.js:543-545`).
    pub(super) async fn finish_via(
        &self,
        h: Handshaken,
        via: Via,
    ) -> Result<PeerConnection, HyperDhtError> {
        let direct = ConnectResult {
            remote_public_key: h.nw_result.remote_public_key,
            server_address: h.relay,
            client_address: h.hs_result.client_address,
            is_relayed: false,
            noise: h.nw_result,
            local_stream_id: h.local_stream_id,
            remote_udx: h.remote_payload.udx,
            udx_socket: None,
        };
        let lease = self.reuse.lease(&via.socket);
        let mut conn = establish_on_stream(&direct, via.stream, via.socket).await?;
        if let Some(lease) = lease {
            conn.hold(lease);
        }
        Ok(conn)
    }
}

#[cfg(test)]
#[path = "connect_route_tests.rs"]
mod tests;
