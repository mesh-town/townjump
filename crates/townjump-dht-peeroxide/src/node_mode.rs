// JS: dht-rpc/index.js (`ephemeral`, `adaptive`, `remoteAddress`): fork `DhtMode` → `NodeMode`, and the node methods that use it.
// See also: dht.rs (the `Dht` trait methods call these), node.rs (the node state).
use crate::error::map_error;
use crate::node::Node;
use peeroxide_dht::rpc::DhtMode;
use std::net::SocketAddrV4;
use townjump_dht::{DhtError, NodeMode};

/// `persistent = !ephemeral`; an address whose host is not a numeric IPv4 becomes `None`.
pub(crate) fn node_mode(mode: &DhtMode) -> NodeMode {
    let address = mode.address.as_ref().and_then(|peer| {
        let host = peer.host.parse().ok()?;
        Some(SocketAddrV4::new(host, peer.port))
    });
    NodeMode {
        persistent: !mode.ephemeral,
        adaptive: mode.adaptive,
        address,
    }
}

impl Node {
    pub fn mode(&self) -> NodeMode {
        node_mode(&self.handle.mode())
    }

    /// Waits until the mode differs from `last`; a closed channel is a destroyed node.
    pub async fn mode_changed(&self, last: NodeMode) -> Result<NodeMode, DhtError> {
        self.check_alive()?;
        let mut watch = self.handle.watch_mode();
        let changed = watch.wait_for(|m| node_mode(m) != last).await;
        let mode = changed.map_err(|_| DhtError::Destroyed)?;
        Ok(node_mode(&mode))
    }

    pub async fn set_adaptive(&self, on: bool) -> Result<(), DhtError> {
        self.check_alive()?;
        self.handle.set_adaptive(on).await.map_err(map_error)
    }
}

#[cfg(test)]
#[path = "node_mode_tests.rs"]
mod tests;
