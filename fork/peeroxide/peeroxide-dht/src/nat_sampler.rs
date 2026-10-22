// JS: hyperdht/lib/nat.js (autoSample)
//! Background NAT sampling: DHT pings out the puncher socket and the bridge
//! that feeds replies on that socket back into the DHT actor.

use crate::messages::Ipv4Peer;
use crate::rpc::DhtHandle;
use libudx::{Datagram, UdxSocket};
use std::time::Duration;
use tokio::sync::mpsc;

/// Bridge raw `dht_reply_rx` datagrams from the puncher socket into the
/// `DhtNode` actor via [`DhtHandle::forward_inbound_reply_bytes`]. Spawned
/// explicitly by the owner of the puncher and left running: replies to
/// `request_via` on the puncher socket keep flowing until the socket's route
/// task ends.
pub(crate) fn spawn_dht_reply_forwarder(
    dht: DhtHandle,
    mut rx: mpsc::UnboundedReceiver<Datagram>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(dgram) = rx.recv().await {
            dht.forward_inbound_reply_bytes(dgram.addr, dgram.data);
        }
    })
}

/// Walks `dht.recent_nodes(max_samples + 5)`, fires up to `max_samples`
/// concurrent `ping_via_socket` calls (each bound by a 2 s timeout) and sends
/// each pong's `(to, from)` into the returned channel. The channel closes when
/// all pings are done (or at once when there are no candidates); dropping the
/// receiver aborts the pings at once, without waiting for the next pong.
pub(crate) fn spawn_sampler(
    dht: &DhtHandle,
    socket: UdxSocket,
    max_samples: usize,
) -> mpsc::UnboundedReceiver<(Ipv4Peer, Ipv4Peer)> {
    let (tx, rx) = mpsc::unbounded_channel();
    let candidates = dht.recent_nodes(max_samples + 5);
    let dht = dht.clone();
    tokio::spawn(async move {
        let mut joinset: tokio::task::JoinSet<
            Result<crate::rpc::PingResponse, crate::rpc::DhtError>,
        > = tokio::task::JoinSet::new();
        for node in candidates.into_iter().take(max_samples) {
            let dht_clone = dht.clone();
            let socket_clone = socket.clone();
            joinset.spawn(async move {
                tokio::time::timeout(
                    Duration::from_millis(2000),
                    dht_clone.ping_via_socket(node, socket_clone),
                )
                .await
                .map_err(|_| crate::rpc::DhtError::ChannelClosed)?
            });
        }
        loop {
            let joined = tokio::select! {
                // Receiver dropped: stop at once instead of waiting for the next pong.
                _ = tx.closed() => {
                    joinset.abort_all();
                    return;
                }
                joined = joinset.join_next() => joined,
            };
            let Some(joined) = joined else {
                return;
            };
            match joined {
                Ok(Ok(resp)) => {
                    let Some(to) = resp.to else {
                        tracing::warn!(from = %format!("{}:{}", resp.from.host, resp.from.port), "auto_sample: ping reply missing reflexive `to`; skipping");
                        continue;
                    };
                    if tx.send((to, resp.from)).is_err() {
                        return;
                    }
                }
                Ok(Err(e)) => {
                    tracing::warn!(err = ?e, "auto_sample: ping_via_socket failed");
                }
                Err(e) => {
                    tracing::warn!(err = %e, "auto_sample: join error");
                }
            }
        }
    });
    rx
}

#[cfg(test)]
#[path = "nat_sampler_tests.rs"]
mod tests;
