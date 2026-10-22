// A blind-relay node: a DHT node that announces itself and pairs the peers that ask it (`relay_through`).
// A node property, not a connection one, so it is a concrete type outside the `Dht` traits; see also factory.rs.
// JS: hyperdht/test/relaying.js:28-43 (RelayServer + createServer + relay.accept)
use crate::bootstrap::BootstrapPlan;
use crate::factory::PeeroxideFactory;
use crate::node::Node;
use peeroxide_dht::blind_relay::BlindRelayServer;
use peeroxide_dht::relay_service::{RelayServiceConfig, run_relay_server};
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;
use townjump_dht::DhtError;
use townjump_keys::KeyPair;

/// How long `close` waits for the relay task to leave its loop before aborting it.
const CLOSE_WAIT: Duration = Duration::from_secs(5);

/// Counters of a relay node (a copy of the fork snapshot; the fork type is not exposed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelayStats {
    /// Sessions open right now.
    pub sessions_active: i64,
    /// Pairing requests received in total.
    pub pairings_requested: u64,
    /// Pairings completed (both sides arrived) in total.
    pub pairings_matched: u64,
}

/// A running relay node. It owns the node's only event channel, so `create_server` is not available on it.
pub struct PeeroxideRelay {
    node: Arc<Node>,
    server: BlindRelayServer,
    task: JoinHandle<()>,
}

impl PeeroxideRelay {
    pub fn public_key(&self) -> [u8; 32] {
        self.node.identity.public_key
    }

    pub fn stats(&self) -> RelayStats {
        let s = self.server.stats();
        RelayStats {
            sessions_active: s.sessions_active,
            pairings_requested: s.pairings_requested,
            pairings_matched: s.pairings_matched,
        }
    }

    /// Destroys the node and waits (up to 5 s) for the relay task to end. The fork's self-announce and sweep
    /// tasks are bare `tokio::spawn`s with no stop handle: they run until the runtime ends.
    pub async fn close(self) {
        let _ = self.node.destroy().await;
        let mut task = self.task;
        if tokio::time::timeout(CLOSE_WAIT, &mut task).await.is_err() {
            task.abort();
        }
    }
}

impl PeeroxideFactory {
    /// Starts a relay node (persistent, not firewalled) and waits for bootstrap: the fork does not retry
    /// a failed self-announce for ten minutes. `identity: None` — a random key pair.
    pub async fn create_relay(
        &self,
        identity: Option<KeyPair>,
    ) -> Result<PeeroxideRelay, DhtError> {
        let plan = BootstrapPlan::parse(self.bootstrap.as_deref())?;
        let mut config = self.node_config(&plan);
        config.dht.ephemeral = Some(false);
        config.dht.firewalled = false;
        let node = Node::start(config, identity).await?;
        node.ready().await?;
        let events = node.take_events()?;
        let (server, task) = run_relay_server(
            node.runtime.handle(),
            node.handle.clone(),
            node.identity.clone(),
            events,
            RelayServiceConfig::default(),
        );
        Ok(PeeroxideRelay { node, server, task })
    }
}

#[cfg(test)]
#[path = "relay_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "relay_js_tests.rs"]
mod js_tests;
