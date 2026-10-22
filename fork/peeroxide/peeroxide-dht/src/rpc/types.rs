// JS: dht-rpc/index.js
//! Public data types: `DhtError`, `DhtConfig`, `PingResponse`, `ResponseData` and the
//! query/request parameter structs. Neighbours: `requests` (user requests), `command` (actor commands).

use std::time::Duration;

use thiserror::Error;

use crate::messages::Ipv4Peer;
use crate::peer::NodeId;

#[cfg(doc)]
use super::handle::DhtHandle;
#[cfg(doc)]
use super::node::spawn;

// ── Error ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
/// Errors returned by [`DhtHandle`] and [`spawn`].
#[non_exhaustive]
pub enum DhtError {
    /// Underlying I/O failed.
    #[error("IO error: {0}")]
    Io(#[from] crate::io::IoError),
    /// The DHT node has been destroyed.
    #[error("node destroyed")]
    Destroyed,
    /// The internal command channel is closed.
    #[error("command channel closed")]
    ChannelClosed,
    /// Bootstrapping did not complete successfully.
    #[error("bootstrap failed")]
    BootstrapFailed,
    /// A request failed with the given message.
    #[error("request failed: {0}")]
    RequestFailed(String),
    /// The node is suspended (`suspend`), or was suspended while the request was waiting.
    #[error("node suspended")]
    Suspended,
}

// ── Public config / request / response types ──────────────────────────────────

/// Configuration for creating a DHT node.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct DhtConfig {
    /// Bootstrap node addresses.
    pub bootstrap: Vec<String>,
    /// Nodes put into the routing table at start without a ping (JS `opts.nodes`).
    pub nodes: Vec<Ipv4Peer>,
    /// Local port to bind.
    pub port: u16,
    /// Local host to bind.
    pub host: String,
    /// Whether to force ephemeral mode.
    pub ephemeral: Option<bool>,
    /// Whether to advertise as firewalled.
    pub firewalled: bool,
    /// Query concurrency limit.
    pub concurrency: usize,
    /// Maximum query window size.
    pub max_window: usize,
    /// Let an ephemeral node become persistent once it is reachable (only when `ephemeral` is unset).
    pub adaptive: bool,
    /// Ticks between adaptive checks; the back-off after a check is three times this.
    pub stable_ticks: u64,
}

/// Current operating mode of a DHT node.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct DhtMode {
    /// True while the node does not serve the routing table.
    pub ephemeral: bool,
    /// True while the node may promote itself to persistent.
    pub adaptive: bool,
    /// True while the node is considered behind a firewall.
    pub firewalled: bool,
    /// Address the table id is derived from while persistent.
    pub address: Option<Ipv4Peer>,
}

impl Default for DhtMode {
    fn default() -> Self {
        Self {
            ephemeral: true,
            adaptive: false,
            firewalled: true,
            address: None,
        }
    }
}

impl Default for DhtConfig {
    fn default() -> Self {
        Self {
            bootstrap: vec![],
            nodes: vec![],
            port: 0,
            host: "0.0.0.0".to_string(),
            ephemeral: None,
            firewalled: true,
            concurrency: 10,
            max_window: 80,
            adaptive: true,
            stable_ticks: super::consts::STABLE_TICKS,
        }
    }
}

#[derive(Debug, Clone)]
/// Response to a ping request.
#[non_exhaustive]
pub struct PingResponse {
    /// Remote peer that replied.
    pub from: Ipv4Peer,
    /// Optional peer id reported by the remote node.
    pub id: Option<NodeId>,
    /// Round-trip time for the ping.
    pub rtt: Duration,
    /// Reflexive address: our address as seen by the remote node.
    pub to: Option<Ipv4Peer>,
    /// Nodes returned by the remote peer (closer nodes from its routing table).
    pub closer_nodes: Vec<Ipv4Peer>,
}

#[derive(Debug, Clone)]
/// Data returned from a DHT request.
#[non_exhaustive]
pub struct ResponseData {
    /// Remote peer that replied.
    pub from: Ipv4Peer,
    /// Optional peer id reported by the remote node.
    pub id: Option<NodeId>,
    /// Optional response token.
    pub token: Option<[u8; 32]>,
    /// Nodes returned by the remote peer.
    pub closer_nodes: Vec<Ipv4Peer>,
    /// Response error code.
    pub error: u64,
    /// Optional response value.
    pub value: Option<Vec<u8>>,
    /// Round-trip time for the request.
    pub rtt: Duration,
    /// Reflexive address: our address as seen by the remote node.
    pub to: Option<Ipv4Peer>,
}

#[derive(Debug, Clone)]
/// Parameters for a user-driven DHT query.
#[non_exhaustive]
pub struct UserQueryParams {
    /// Query target node id.
    pub target: NodeId,
    /// RPC command to send.
    pub command: u64,
    /// Optional query payload.
    pub value: Option<Vec<u8>>,
    /// Whether the query is a commit.
    pub commit: bool,
    /// Optional per-query concurrency override.
    pub concurrency: Option<usize>,
}

#[derive(Debug, Clone, Default)]
#[non_exhaustive]
/// Parameters for a user-driven DHT request.
pub struct UserRequestParams {
    /// Optional request token.
    pub token: Option<[u8; 32]>,
    /// RPC command to send.
    pub command: u64,
    /// Optional target node id.
    pub target: Option<NodeId>,
    /// Optional request payload.
    pub value: Option<Vec<u8>>,
    /// Optional per-request timeout override (ms). Defaults to the dht-rpc DEFAULT_TIMEOUT_MS.
    pub timeout_ms: Option<u64>,
    /// Optional per-request retries override. Defaults to the dht-rpc DEFAULT_RETRIES.
    pub retries: Option<u32>,
}

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
