// JS: `new HyperDHT({ keyPair })` (hyperdht/index.js), testnet bootstrap — `TOWNJUMP_BOOTSTRAP`.
// See also: dht.rs (the created node), error.rs (fork error mapping).
use crate::bootstrap::BootstrapPlan;
use crate::dht::PeeroxideDht;
use crate::node::Node;
use crate::nodes::to_peers;
use peeroxide_dht::hyperdht::HyperDhtConfig;
use std::net::SocketAddrV4;
use townjump_dht::{DhtError, DhtFactory};
use townjump_keys::KeyPair;

/// Creates DHT nodes on the peeroxide fork.
#[derive(Debug, Clone)]
pub struct PeeroxideFactory {
    /// `None` — `DEFAULT_BOOTSTRAP`; if all addresses are loopback, the host is `127.0.0.1`.
    pub bootstrap: Option<Vec<String>>,
    /// Adaptive mode (the node promotes itself to persistent when reachable); default `true`.
    pub adaptive: bool,
    /// Nodes put into the routing table without a ping (dht-rpc `opts.nodes`); default empty.
    pub nodes: Vec<SocketAddrV4>,
}

impl Default for PeeroxideFactory {
    fn default() -> Self {
        PeeroxideFactory {
            bootstrap: None,
            adaptive: true,
            nodes: Vec::new(),
        }
    }
}

impl PeeroxideFactory {
    /// Reads `TOWNJUMP_BOOTSTRAP` (`host:port,…`) and `TOWNJUMP_DHT_ADAPTIVE` (`0`/`false`/`off`/`no` — off);
    /// a malformed bootstrap is an error later, in `create`.
    pub fn from_env() -> PeeroxideFactory {
        Self::from_env_with(|name| std::env::var(name).ok())
    }

    /// Pure part of `from_env`: the variable value (`None` — unset). An empty string counts as unset (`None` — the fork
    /// default); otherwise addresses are split on `,` in the original order, unchecked — `create` rejects a malformed address.
    pub fn from_env_value(value: Option<&str>) -> PeeroxideFactory {
        let value = value.map(str::to_string);
        Self::from_env_with(|name| value.clone().filter(|_| name == BOOTSTRAP_VAR))
    }

    /// Reads every variable through `get(name)`, so tests need no process environment.
    pub fn from_env_with(get: impl Fn(&str) -> Option<String>) -> PeeroxideFactory {
        let list = get(BOOTSTRAP_VAR).filter(|v| !v.is_empty());
        let bootstrap = list.map(|v| v.split(',').map(str::to_string).collect());
        let adaptive = adaptive_from_env(get(ADAPTIVE_VAR).as_deref());
        PeeroxideFactory {
            bootstrap,
            adaptive,
            nodes: Vec::new(),
        }
    }
}

impl PeeroxideFactory {
    /// Fork config for `plan`; `ephemeral` stays as the plan sets it (`None` for the public network, `Some(true)` on loopback).
    pub(crate) fn node_config(&self, plan: &BootstrapPlan) -> HyperDhtConfig {
        let mut config = plan.config();
        config.dht.adaptive = self.adaptive;
        config.dht.nodes = to_peers(&self.nodes);
        config
    }
}

const BOOTSTRAP_VAR: &str = "TOWNJUMP_BOOTSTRAP";
const ADAPTIVE_VAR: &str = "TOWNJUMP_DHT_ADAPTIVE";

/// `0`, `false`, `off`, `no` (any case, trimmed) switch adaptive mode off; anything else, and unset, leaves it on.
pub(crate) fn adaptive_from_env(value: Option<&str>) -> bool {
    let Some(value) = value else { return true };
    let off = ["0", "false", "off", "no"];
    !off.contains(&value.trim().to_lowercase().as_str())
}

impl DhtFactory for PeeroxideFactory {
    type Dht = PeeroxideDht;

    /// A malformed bootstrap is an error before the node starts; the node itself does not wait for bootstrap (like `new HyperDHT`).
    async fn create(&self, identity: Option<KeyPair>) -> Result<PeeroxideDht, DhtError> {
        let plan = BootstrapPlan::parse(self.bootstrap.as_deref())?;
        let node = Node::start(self.node_config(&plan), identity).await?;
        Ok(PeeroxideDht::new(node))
    }

    fn set_nodes(&mut self, nodes: Vec<SocketAddrV4>) {
        self.nodes = nodes;
    }
}

#[cfg(test)]
#[path = "factory_tests.rs"]
mod tests;
