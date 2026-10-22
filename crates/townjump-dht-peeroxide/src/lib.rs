// JS: none (adapter over the peeroxide fork: `HyperDHT` from hyperdht 6.34.0 → townjump-dht traits).
// The only crate that sees peeroxide-dht and libudx; the rest of the code knows only townjump-dht.
// Files: factory (node creation) → node (shared node state; node_suspend — suspend/resume/adopt) → dht, server (traits),
// stream (stream) ← slot (connection owner) ← tunnels (registry for teardown on suspend), error, bootstrap.
mod bootstrap;
pub mod dht;
pub mod error;
pub mod factory;
mod node;
mod node_mode;
mod node_suspend;
mod nodes;
mod relay;
pub mod server;
mod slot;
pub mod stream;
mod tunnels;

pub use dht::PeeroxideDht;
pub use factory::PeeroxideFactory;
pub use relay::{PeeroxideRelay, RelayStats};
pub use server::PeeroxideServer;

#[cfg(test)]
mod js_peer_support_tests;
#[cfg(test)]
#[path = "nodes_live_tests.rs"]
mod nodes_live_tests;
#[cfg(test)]
mod opts_tests;
#[cfg(test)]
mod tunnel_live_tests;
