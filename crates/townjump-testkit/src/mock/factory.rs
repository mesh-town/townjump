// JS: `require('hyperdht')` → `FakeDHT` from mock-dht.cjs; here the factory is one in-memory network with its own trace.
// Related: dht.rs (nodes), shared.rs (network state), config.rs (settings).
use crate::lock::lock;
use crate::mock::config::MockConfig;
use crate::mock::dht::MockDht;
use crate::mock::events;
use crate::mock::shared::Shared;
use crate::mock::trace::MockTrace;
use std::net::SocketAddrV4;
use std::sync::Arc;
use townjump_dht::{DhtError, DhtFactory, KeyPair};

/// Identity filler of an anonymous node; the first 8 bytes are the node number.
const ANONYMOUS_FILL: u8 = 0xA0;

/// Mock factory: clones share the network, two `MockFactory::new()` are two separate networks.
#[derive(Clone, Default)]
pub struct MockFactory {
    shared: Arc<Shared>,
}

impl MockFactory {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_config(config: MockConfig) -> Self {
        MockFactory {
            shared: Arc::new(Shared::new(config)),
        }
    }

    /// Trace of calls of all nodes of this factory.
    pub fn trace(&self) -> MockTrace {
        self.shared.trace.clone()
    }

    /// Nodes passed to `set_nodes` (`None` — it was never called).
    pub fn seeded_nodes(&self) -> Option<Vec<SocketAddrV4>> {
        lock(&self.shared.seeded).clone()
    }

    /// Opens the `hold_listen` gate: waiting and future `listen` calls pass. Without a gate - nothing.
    pub fn release_listen(&self) {
        self.shared.switches.release_listen();
    }

    /// Opens the `hold_put` gate: waiting and future `mutable_put` calls pass. Without a gate - nothing.
    pub fn release_put(&self) {
        self.shared.switches.release_put();
    }

    /// How many times `mutable_put` was called on nodes of this factory, including those stuck on the gate and failed ones.
    pub fn put_calls(&self) -> usize {
        self.shared.switches.put_calls()
    }
}

fn anonymous_key(number: u64) -> [u8; 32] {
    let mut key = [ANONYMOUS_FILL; 32];
    key[..8].copy_from_slice(&number.to_be_bytes());
    key
}

impl DhtFactory for MockFactory {
    type Dht = MockDht;

    async fn create(&self, identity: Option<KeyPair>) -> Result<MockDht, DhtError> {
        let known = identity.as_ref().map(|kp| &kp.public_key);
        self.shared.emit(events::new_event(known));
        let key = known
            .copied()
            .unwrap_or_else(|| anonymous_key(self.shared.next_anonymous()));
        Ok(MockDht::new(self.shared.clone(), key))
    }

    fn set_nodes(&mut self, nodes: Vec<SocketAddrV4>) {
        *lock(&self.shared.seeded) = Some(nodes);
    }
}

#[cfg(test)]
#[path = "network_tests.rs"]
mod tests;
