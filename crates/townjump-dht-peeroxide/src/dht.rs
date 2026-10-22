// JS: the `HyperDHT` node (connect, mutablePut, mutableGet, destroy, suspend, resume, createServer).
// See also: factory.rs creates the node, node.rs holds its state, server.rs — the node server, stream.rs — the connection stream.
use crate::error::map_error;
use crate::node::{Node, fork_pair};
use crate::server::PeeroxideServer;
use peeroxide_dht::hyperdht::{ConnectOpts as ForkConnectOpts, RelayThrough};
use std::fmt;
use std::net::SocketAddrV4;
use std::sync::Arc;
use townjump_dht::{
    ConnectOpts, Dht, DhtConnection, DhtError, Incoming, MutableRecord, NodeMode, ServerOpts,
};
use townjump_keys::KeyPair;

/// holesail connect options → fork options; `local_connection` stays the fork default.
pub(crate) fn fork_connect_opts(opts: ConnectOpts) -> ForkConnectOpts {
    let mut fork = ForkConnectOpts::default();
    fork.reusable_socket = opts.reusable_socket;
    fork.relay_through = opts.relay_through.map(RelayThrough::Key);
    fork
}

/// DHT node; one `UdxRuntime` per node (in `Node`).
pub struct PeeroxideDht {
    node: Arc<Node>,
}

impl PeeroxideDht {
    pub(crate) fn new(node: Arc<Node>) -> PeeroxideDht {
        PeeroxideDht { node }
    }
}

impl fmt::Debug for PeeroxideDht {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PeeroxideDht").finish_non_exhaustive()
    }
}

impl Dht for PeeroxideDht {
    type Server = PeeroxideServer;

    async fn to_array(&self, limit: Option<usize>) -> Result<Vec<SocketAddrV4>, DhtError> {
        self.node.to_array(limit).await
    }

    fn create_server(&self, opts: ServerOpts) -> Result<(PeeroxideServer, Incoming), DhtError> {
        PeeroxideServer::create(Arc::clone(&self.node), opts)
    }

    /// `opts.reusable_socket` — retry along the route of the previous connection to the same server (JS `reusableSocket`).
    async fn connect(
        &self,
        remote: [u8; 32],
        opts: ConnectOpts,
    ) -> Result<DhtConnection, DhtError> {
        self.node.ready().await?;
        let node = &self.node;
        let o = fork_connect_opts(opts);
        let connected =
            node.handle
                .connect_with_options(&node.identity, remote, &[], &node.runtime, o);
        node.adopt(connected.await.map_err(map_error)?)
    }

    async fn mutable_put(
        &self,
        kp: &KeyPair,
        value: &[u8],
        seq: Option<u64>,
    ) -> Result<u64, DhtError> {
        self.node.ready().await?;
        let pair = fork_pair(kp);
        let put = self.node.handle.mutable_put(&pair, value, seq.unwrap_or(0));
        Ok(put.await.map_err(map_error)?.seq)
    }

    /// `latest` is not read: the fork always returns the highest `seq`.
    async fn mutable_get(
        &self,
        pk: [u8; 32],
        latest: Option<bool>,
    ) -> Result<Option<MutableRecord>, DhtError> {
        let _ = latest;
        self.node.ready().await?;
        let got = self
            .node
            .handle
            .mutable_get(&pk, 0)
            .await
            .map_err(map_error)?;
        Ok(got.map(|r| MutableRecord {
            seq: r.seq,
            value: r.value,
        }))
    }

    async fn destroy(&self) -> Result<(), DhtError> {
        self.node.destroy().await
    }

    /// Like `dht.suspend()`: servers go silent, issued tunnels are torn down (`Node::suspend`).
    async fn suspend(&self) -> Result<(), DhtError> {
        self.node.suspend().await
    }

    async fn resume(&self) -> Result<(), DhtError> {
        self.node.resume().await
    }

    fn node_mode(&self) -> NodeMode {
        self.node.mode()
    }

    async fn mode_changed(&self, last: NodeMode) -> Result<NodeMode, DhtError> {
        self.node.mode_changed(last).await
    }

    async fn set_adaptive(&self, on: bool) -> Result<(), DhtError> {
        self.node.set_adaptive(on).await
    }
}

#[cfg(test)]
#[path = "dht_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "dht_lifecycle_tests.rs"]
mod lifecycle_tests;
