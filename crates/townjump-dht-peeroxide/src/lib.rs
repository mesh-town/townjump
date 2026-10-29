// JS: none (adapter over the peeroxide fork: `HyperDHT` from hyperdht 6.34.0 → townjump-dht traits).
// The only crate that sees peeroxide-dht and libudx; the rest of the code knows only townjump-dht.
// Files: factory (node creation) → node (shared node state; node_suspend — suspend/resume/adopt) → dht, server (traits),
// stream (stream) ← slot (connection owner) ← tunnels (registry for teardown on suspend), error, bootstrap;
// relay (relay node) with relay_opts (its options), path (relay/direct label of a connection).
mod bootstrap;
pub mod dht;
pub mod error;
pub mod factory;
mod node;
mod node_mode;
mod node_suspend;
mod nodes;
mod opts_map;
mod path;
mod relay;
mod relay_opts;
pub mod server;
mod slot;
pub mod stream;
mod tunnels;

pub use dht::PeeroxideDht;
pub use factory::PeeroxideFactory;
pub use relay::{PeeroxideRelay, RelayStats};
pub use relay_opts::{NatKind, RelayOpts, RelayReachability};
pub use server::PeeroxideServer;

#[cfg(test)]
mod js_peer_support_tests;
#[cfg(test)]
mod native_testnet_tests;
#[cfg(test)]
#[path = "nodes_live_tests.rs"]
mod nodes_live_tests;
#[cfg(test)]
mod opts_tests;
#[cfg(test)]
mod tunnel_live_tests;
