// JS: dht-rpc/index.js (`ephemeral`, `adaptive`, `remoteAddress`) — the node mode as seen from outside.
// See also: traits.rs (`Dht::node_mode`, `mode_changed`, `set_adaptive`).
use std::net::SocketAddrV4;

/// Operating mode of a DHT node. Default: ephemeral, not adaptive, no address — what a backend without modes reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NodeMode {
    /// The node serves the routing table (opposite of ephemeral).
    pub persistent: bool,
    /// The node may promote itself to persistent on its own.
    pub adaptive: bool,
    /// Public address the node id is derived from while persistent.
    pub address: Option<SocketAddrV4>,
}
