// A blind-relay node: a DHT node that pairs the peers that ask it (`relay_through`). Options (relay_opts.rs):
// admission by peer key, a persistent key, self-announce on/off, a fixed port and a reachability check.
// A node property, not a connection one, so it is a concrete type outside the `Dht` traits; see also factory.rs.
// JS: hyperdht/test/relaying.js:28-43 (RelayServer + createServer + relay.accept)
use crate::bootstrap::BootstrapPlan;
use crate::error::map_error;
use crate::factory::PeeroxideFactory;
use crate::node::Node;
use crate::relay_opts::{
    NatKind, RelayOpts, RelayReachability, publish_address, relay_node_config, service_config,
};
use peeroxide_dht::blind_relay::BlindRelayServer;
use peeroxide_dht::hyperdht::{HyperDhtConfig, ServerEvent};
use peeroxide_dht::relay_service::{RelayServiceHandle, run_relay_server};
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedReceiver;
use townjump_dht::{DhtError, RelayAddress};
use townjump_keys::KeyPair;

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
/// Dropping it aborts the relay tasks and destroys the node in the background (no unannounce): use `close`
/// for an orderly stop.
pub struct PeeroxideRelay {
    node: Arc<Node>,
    server: BlindRelayServer,
    service: Option<RelayServiceHandle>,
    /// Port of the server socket: read once at start, the relay node is never suspended.
    port: u16,
}

impl PeeroxideRelay {
    pub fn public_key(&self) -> [u8; 32] {
        self.node.identity.public_key
    }

    /// Counters keep their values after `close`.
    pub fn stats(&self) -> RelayStats {
        let s = self.server.stats();
        RelayStats {
            sessions_active: s.sessions_active,
            pairings_requested: s.pairings_requested,
            pairings_matched: s.pairings_matched,
        }
    }

    /// UDP port the relay listens on (`RelayOpts.port`, or the one picked when that was `0`).
    pub fn local_port(&self) -> u16 {
        self.port
    }

    /// NAT type and the address worth publishing. With `assume_open: true` it is always `Open` (no check is made),
    /// and the address is known only when the node host is not a wildcard (`None` on `0.0.0.0`).
    /// With `assume_open: false` it turns `Open` after the node proves itself reachable (about 10-15 s after
    /// bootstrap); a failed check is not repeated, as in the fork and JS: if it never becomes `Open`, recreate the relay.
    pub async fn reachability(&self) -> Result<RelayReachability, DhtError> {
        self.node.check_alive()?;
        let handle = &self.node.handle;
        let firewall = NatKind::from(handle.firewalled().await.map_err(map_error)?);
        let declared = handle.mode().address;
        let observed = handle.remote_address().await.map_err(map_error)?;
        Ok(RelayReachability {
            local_port: self.port,
            public_address: publish_address(firewall, declared, observed),
            firewall,
        })
    }

    /// Relay key plus the public address from `reachability`; `None` while that address is unknown
    /// (see `reachability` for when it is known at all).
    pub async fn relay_address(&self) -> Result<Option<RelayAddress>, DhtError> {
        let public_key = self.public_key();
        let reach = self.reachability().await?;
        Ok(reach
            .public_address
            .map(|addr| RelayAddress { public_key, addr }))
    }

    /// Stops the relay service (all its tasks; the announce, if any, is withdrawn while the node is alive,
    /// within about 6 s), then destroys the node and frees its port.
    pub async fn close(mut self) {
        if let Some(service) = self.service.take() {
            service.close().await;
        }
        let _ = self.node.destroy().await;
    }
}

impl Drop for PeeroxideRelay {
    /// The service handle aborts its tasks on its own drop; the node is destroyed in the background if a
    /// runtime is there. No network calls for the announce.
    fn drop(&mut self) {
        if self.node.check_alive().is_err() {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let node = self.node.clone();
        runtime.spawn(async move {
            let _ = node.destroy().await;
        });
    }
}

impl PeeroxideFactory {
    /// Starts a relay node with `opts` and waits for bootstrap: the fork does not retry a failed self-announce
    /// for ten minutes. A busy `port` or any start error is a `DhtError`; the node is destroyed on error.
    pub async fn create_relay_with(&self, opts: RelayOpts) -> Result<PeeroxideRelay, DhtError> {
        let plan = BootstrapPlan::parse(self.bootstrap.as_deref())?;
        start_relay(self.node_config(&plan), opts).await
    }

    /// `create_relay_with` with default options: a self-announced relay, assumed open, everyone admitted.
    /// `identity: None` — a random key pair.
    pub async fn create_relay(
        &self,
        identity: Option<KeyPair>,
    ) -> Result<PeeroxideRelay, DhtError> {
        let opts = RelayOpts {
            identity,
            ..Default::default()
        };
        self.create_relay_with(opts).await
    }
}

/// `create_relay_with` on a ready base config (tests bind a loopback-network relay to a LAN address through it).
pub(crate) async fn start_relay(
    base: HyperDhtConfig,
    opts: RelayOpts,
) -> Result<PeeroxideRelay, DhtError> {
    let config = relay_node_config(base, &opts);
    let node = Node::start(config, opts.identity.clone()).await?;
    let (port, events) = match ready_port(&node).await {
        Ok(started) => started,
        Err(e) => {
            let _ = node.destroy().await;
            return Err(e);
        }
    };
    let (server, service) = run_relay_server(
        node.runtime.handle(),
        node.handle.clone(),
        node.identity.clone(),
        events,
        service_config(&opts),
    );
    Ok(PeeroxideRelay {
        node,
        server,
        service: Some(service),
        port,
    })
}

/// Waits for bootstrap, then reads the server socket port and takes the node's event channel.
async fn ready_port(node: &Node) -> Result<(u16, UnboundedReceiver<ServerEvent>), DhtError> {
    node.ready().await?;
    let port = node.handle.local_port().await.map_err(map_error)?;
    Ok((port, node.take_events()?))
}

#[cfg(test)]
#[path = "relay_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "relay_js_tests.rs"]
mod js_tests;

#[cfg(test)]
#[path = "relay_admit_tests.rs"]
mod relay_admit_tests;

#[cfg(test)]
#[path = "relay_restart_tests.rs"]
mod relay_restart_tests;

#[cfg(test)]
#[path = "relay_reach_tests.rs"]
mod relay_reach_tests;
