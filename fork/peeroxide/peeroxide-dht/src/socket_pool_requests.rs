//! Request lane of a pooled socket: internal PING requests that reach the socket before
//! the DHT actor is attached are queued and then handed to the actor with the socket as
//! the reply path, so the answer leaves from the same socket.

use std::net::SocketAddr;

use libudx::UdxSocket;
use tokio::sync::mpsc;

use crate::rpc::DhtHandle;

/// Capacity of the per-socket ping queue while no DHT actor is attached.
pub(crate) const DHT_REQUEST_QUEUE: usize = 16;

/// An internal PING request that reached a pooled socket: sender, raw bytes and a clone of
/// the socket to answer through.
pub(crate) struct DhtRequestSeen {
    pub(crate) addr: SocketAddr,
    pub(crate) data: Vec<u8>,
    pub(crate) via: UdxSocket,
}

/// Forwards each queued request to the actor. Holds no socket clone of its own: the clone
/// travels in every item, so the socket can close once the owner is gone.
pub(crate) fn spawn_request_forwarder(
    dht: DhtHandle,
    mut rx: mpsc::Receiver<DhtRequestSeen>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(seen) = rx.recv().await {
            tracing::debug!(addr = %seen.addr, "socket_pool: DHT ping on holepunch socket handed to the DHT actor");
            dht.forward_inbound_via(seen.addr, seen.data, seen.via);
        }
    })
}
