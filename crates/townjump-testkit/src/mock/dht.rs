// JS: `FakeDHT` from mock-dht.cjs (events, `mutableGet/Put`) and hyperdht/lib/hyperdht.js (`connect`, `destroy`, `suspend`).
// Related: server.rs - the node server, shared.rs - the server registry and network records, factory.rs creates nodes.
use crate::lock::lock;
use crate::mock::events;
use crate::mock::server::MockServer;
use crate::mock::shared::Shared;
use crate::mock::stub::StubStream;
use std::fmt;
use std::net::SocketAddrV4;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use townjump_dht::{
    ConnectOpts, Dht, DhtConnection, DhtError, Incoming, KeyPair, MutableRecord, ServerOpts,
};

/// `duplex` buffer per direction: writing 1 MiB without a reader does not block.
const DUPLEX_BUFFER: usize = 1 << 20;
/// Server incoming queue.
const INCOMING_CAPACITY: usize = 64;

/// Node state shared by `MockDht` and its servers.
#[derive(Default)]
pub(crate) struct NodeState {
    suspended: AtomicBool,
    destroyed: AtomicBool,
}

impl NodeState {
    pub(crate) fn is_suspended(&self) -> bool {
        self.suspended.load(Ordering::SeqCst)
    }

    pub(crate) fn is_destroyed(&self) -> bool {
        self.destroyed.load(Ordering::SeqCst)
    }
}

/// Mock node: the network is shared with the other nodes of the same factory.
pub struct MockDht {
    shared: Arc<Shared>,
    node: Arc<NodeState>,
    identity: [u8; 32],
    servers: Mutex<Vec<MockServer>>,
}

impl fmt::Debug for MockDht {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MockDht")
            .field("identity", &self.identity)
            .finish_non_exhaustive()
    }
}

impl MockDht {
    pub(crate) fn new(shared: Arc<Shared>, identity: [u8; 32]) -> Self {
        MockDht {
            shared,
            node: Arc::new(NodeState::default()),
            identity,
            servers: Mutex::new(Vec::new()),
        }
    }

    /// Node identity: the key of the pair `create(Some(kp))` or a synthetic key `create(None)`.
    pub fn public_key(&self) -> [u8; 32] {
        self.identity
    }

    fn ensure_alive(&self) -> Result<(), DhtError> {
        if self.node.is_destroyed() {
            return Err(DhtError::Destroyed);
        }
        Ok(())
    }

    /// The key is not in the registry: lenient mode yields a stub, strict mode `PeerNotFound`.
    fn connect_unregistered(&self, remote: [u8; 32]) -> Result<DhtConnection, DhtError> {
        if !self.shared.config.lenient_connect {
            return Err(DhtError::PeerNotFound);
        }
        Ok(DhtConnection::new(remote, Box::new(StubStream)).silent_close())
    }

    fn close_own_servers(&self) {
        let servers = std::mem::take(&mut *lock(&self.servers));
        servers.iter().for_each(MockServer::shut);
    }
}

impl Dht for MockDht {
    type Server = MockServer;

    fn create_server(&self, opts: ServerOpts) -> Result<(MockServer, Incoming), DhtError> {
        self.shared.emit(events::create_server_event(&opts));
        self.ensure_alive()?;
        let (tx, incoming) = Incoming::channel(INCOMING_CAPACITY);
        let node = self.node.clone();
        let server = MockServer::new(self.shared.clone(), node, opts.firewall, tx);
        lock(&self.servers).push(server.clone());
        Ok((server, incoming))
    }

    async fn connect(
        &self,
        remote: [u8; 32],
        opts: ConnectOpts,
    ) -> Result<DhtConnection, DhtError> {
        self.shared.emit(events::connect_event(&remote, &opts));
        self.ensure_alive()?;
        let Some(server) = self.shared.server(&remote) else {
            return self.connect_unregistered(remote);
        };
        server.admit(&self.identity)?;
        let (near, far) = tokio::io::duplex(DUPLEX_BUFFER);
        let incoming = DhtConnection::new(self.identity, Box::new(far));
        server.deliver(incoming).await?;
        Ok(DhtConnection::new(remote, Box::new(near)))
    }

    async fn mutable_put(
        &self,
        kp: &KeyPair,
        value: &[u8],
        seq: Option<u64>,
    ) -> Result<u64, DhtError> {
        self.shared.switches.before_put().await?;
        self.shared
            .emit(events::mutable_put_event(&kp.public_key, value, seq));
        self.ensure_alive()?;
        let seq = seq.unwrap_or(0);
        self.shared.store(kp.public_key, seq, value.to_vec());
        Ok(seq)
    }

    async fn mutable_get(
        &self,
        pk: [u8; 32],
        latest: Option<bool>,
    ) -> Result<Option<MutableRecord>, DhtError> {
        self.shared.emit(events::mutable_get_event(&pk, latest));
        self.ensure_alive()?;
        Ok(self.shared.find_record(&pk))
    }

    async fn destroy(&self) -> Result<(), DhtError> {
        self.shared.emit(events::destroy_event());
        if !self.node.destroyed.swap(true, Ordering::SeqCst) {
            self.close_own_servers();
        }
        Ok(())
    }

    async fn suspend(&self) -> Result<(), DhtError> {
        self.shared.emit(events::suspend_event());
        self.node.suspended.store(true, Ordering::SeqCst);
        Ok(())
    }

    async fn resume(&self) -> Result<(), DhtError> {
        self.shared.emit(events::resume_event());
        self.node.suspended.store(false, Ordering::SeqCst);
        Ok(())
    }

    /// The nodes given to `set_nodes`, at most `limit` of them.
    async fn to_array(&self, limit: Option<usize>) -> Result<Vec<SocketAddrV4>, DhtError> {
        self.ensure_alive()?;
        let seeded = lock(&self.shared.seeded).clone().unwrap_or_default();
        Ok(seeded
            .into_iter()
            .take(limit.unwrap_or(usize::MAX))
            .collect())
    }
}

#[cfg(test)]
#[path = "dht_tests.rs"]
mod tests;
