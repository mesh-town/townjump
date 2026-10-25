// JS: `new HyperDHT({ keyPair })`, `dht.createServer`, `dht.connect`, `mutablePut`, `mutableGet`, `destroy`, `suspend`, `resume`, `server.listen/close`.
// Related: types.rs (arguments), incoming.rs (incoming), error.rs (`DhtError`). Static dispatch, no `async-trait`.
use crate::{
    error::DhtError,
    incoming::Incoming,
    mode::NodeMode,
    types::{ConnectOpts, DhtConnection, MutableRecord, ServerOpts},
};
use std::net::SocketAddrV4;
use townjump_keys::KeyPair;

/// Creates a DHT node: `new HyperDHT({ keyPair })`.
pub trait DhtFactory: Send + Sync + 'static {
    type Dht: Dht;
    /// `identity: None` — a random one-off node key pair.
    fn create(
        &self,
        identity: Option<KeyPair>,
    ) -> impl Future<Output = Result<Self::Dht, DhtError>> + Send;
    /// Nodes the next `create` puts into its routing table without a ping (dht-rpc `opts.nodes`); ignored by backends without a table.
    fn set_nodes(&mut self, _nodes: Vec<SocketAddrV4>) {}
}

/// DHT node.
pub trait Dht: Send + Sync + 'static {
    type Server: DhtServer;
    fn create_server(&self, opts: ServerOpts) -> Result<(Self::Server, Incoming), DhtError>;
    fn connect(
        &self,
        remote: [u8; 32],
        opts: ConnectOpts,
    ) -> impl Future<Output = Result<DhtConnection, DhtError>> + Send;
    /// `seq: None` → 0; returns the `seq` that was written.
    fn mutable_put(
        &self,
        kp: &KeyPair,
        value: &[u8],
        seq: Option<u64>,
    ) -> impl Future<Output = Result<u64, DhtError>> + Send;
    /// Record with the highest `seq` among the responders (`latest` by default, as in JS).
    fn mutable_get(
        &self,
        pk: [u8; 32],
        latest: Option<bool>,
    ) -> impl Future<Output = Result<Option<MutableRecord>, DhtError>> + Send;
    fn destroy(&self) -> impl Future<Output = Result<(), DhtError>> + Send;
    /// `dht.suspend()`: the node stops answering and polling the network; the fork needs `suspend`.
    fn suspend(&self) -> impl Future<Output = Result<(), DhtError>> + Send;
    /// `dht.resume()`: the inverse of `suspend`.
    fn resume(&self) -> impl Future<Output = Result<(), DhtError>> + Send;
    /// Current node mode; backends without modes report the default (ephemeral, not adaptive, firewalled).
    fn node_mode(&self) -> NodeMode {
        NodeMode::default()
    }
    /// Resolves with the new mode once it differs from `last`; never resolves for backends without modes.
    fn mode_changed(
        &self,
        _last: NodeMode,
    ) -> impl Future<Output = Result<NodeMode, DhtError>> + Send {
        std::future::pending()
    }
    /// Turns adaptive mode on or off (extension over JS, where it is a constructor flag only).
    fn set_adaptive(&self, _on: bool) -> impl Future<Output = Result<(), DhtError>> + Send {
        std::future::ready(Err(DhtError::Backend(
            "adaptive mode is not supported".to_string(),
        )))
    }
    /// Connects to `remote` at a known address of its DHT socket (`IP:local_port()` of the remote node), without
    /// waiting for bootstrap and without a DHT lookup; default options (no relay, no reusable socket). Fails if
    /// nobody answers there (after the request timeout) or the node there serves no such key.
    fn connect_direct(
        &self,
        _remote: [u8; 32],
        _addr: SocketAddrV4,
    ) -> impl Future<Output = Result<DhtConnection, DhtError>> + Send {
        std::future::ready(Err(DhtError::Backend(
            "direct connect is not supported".to_string(),
        )))
    }
    /// Port of the node's DHT socket, the one `connect_direct` of a peer must target.
    fn local_port(&self) -> impl Future<Output = Result<u16, DhtError>> + Send {
        std::future::ready(Err(DhtError::Backend(
            "local port is not supported".to_string(),
        )))
    }
    /// Routing table export, as dht-rpc `toArray()`: freshest first, host and port only; empty for backends without a table.
    fn to_array(
        &self,
        _limit: Option<usize>,
    ) -> impl Future<Output = Result<Vec<SocketAddrV4>, DhtError>> + Send {
        std::future::ready(Ok(Vec::new()))
    }
}

/// Node server: `Clone`, so it can be held by both the accept task and the owner.
pub trait DhtServer: Clone + Send + Sync + 'static {
    /// Announce `hash(pk)` and accept incoming connections.
    fn listen(&self, kp: KeyPair) -> impl Future<Output = Result<(), DhtError>> + Send;
    fn close(&self) -> impl Future<Output = Result<(), DhtError>> + Send;
}

#[cfg(test)]
#[path = "traits_tests.rs"]
mod tests;
