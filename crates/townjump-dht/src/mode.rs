// JS: dht-rpc/index.js (`ephemeral`, `adaptive`, `remoteAddress`) — the node mode as seen from outside.
// See also: traits.rs (`Dht::node_mode`, `mode_changed`, `set_adaptive`).
use std::net::SocketAddrV4;

/// Operating mode of a DHT node. Default: ephemeral, not adaptive, firewalled, no address — what a backend without
/// modes reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeMode {
    /// The node serves the routing table (opposite of ephemeral).
    pub persistent: bool,
    /// The node may promote itself to persistent on its own.
    pub adaptive: bool,
    /// `true` while the node considers itself behind a firewall (dht-rpc `firewalled`): peers can reach it only
    /// through holepunching or a relay. `false` once the node has verified it is reachable from outside, and on a
    /// loopback-only network by configuration.
    pub firewalled: bool,
    /// Public address the node id is derived from while persistent.
    pub address: Option<SocketAddrV4>,
}

impl Default for NodeMode {
    fn default() -> Self {
        NodeMode {
            persistent: false,
            adaptive: false,
            firewalled: true,
            address: None,
        }
    }
}

#[cfg(test)]
#[path = "mode_tests.rs"]
mod tests;
