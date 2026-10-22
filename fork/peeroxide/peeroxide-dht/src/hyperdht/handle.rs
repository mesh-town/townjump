//! `HyperDhtHandle`: the cloneable handle and its accessors (stats, sockets, addresses, server registry).
//! Neighbours: the API is spread over `lookup`, `announce`, `records`, `connect*`; `node` builds the handle.

use std::sync::{Arc, Mutex};

use tokio::sync::{mpsc, oneshot};

use libudx::UdxSocket;

use crate::messages::Ipv4Peer;
use crate::persistent::PersistentStats;
use crate::router::{ForwardEntry, Router};
use crate::rpc::DhtHandle;

use super::Freeze;
use super::relay_cache::RelayCache;
use super::reuse::Reuse;
use super::types::{HyperDhtError, ServerEvent};

// ── AdminRequest ──────────────────────────────────────────────────────────────

pub(super) enum AdminRequest {
    PersistentStats {
        reply: oneshot::Sender<PersistentStats>,
    },
}

// ── HyperDhtHandle ────────────────────────────────────────────────────────────

#[derive(Clone)]
/// Main public HyperDHT API handle.
pub struct HyperDhtHandle {
    pub(super) dht: DhtHandle,
    pub(super) router: Arc<Mutex<Router>>,
    pub(super) server_tx: mpsc::UnboundedSender<ServerEvent>,
    pub(super) admin_tx: mpsc::UnboundedSender<AdminRequest>,
    pub(super) current_relay_addresses: Arc<Mutex<Vec<Ipv4Peer>>>,
    /// Relay addresses of the last connect per target key (JS `_relayAddressesCache`).
    pub(super) relay_cache: Arc<Mutex<RelayCache>>,
    pub(super) freezers: Arc<Mutex<Vec<mpsc::UnboundedSender<Freeze>>>>,
    /// Held sockets and routes shared by client and server.
    pub(crate) reuse: Reuse,
}

impl HyperDhtHandle {
    // ── WIRE STATS ────────────────────────────────────────────────────────────

    /// Snapshot of cumulative wire bytes (sent, received) since this DHT
    /// node started. Counts every UDP datagram exchanged at the IO layer
    /// — queries, requests, replies, retries, relays, and any user-issued
    /// puts/gets — regardless of which higher-level operation produced them.
    ///
    /// Useful for distinguishing "useful payload throughput" (what consumers
    /// see) from "raw network throughput" (what the OS sees). The ratio
    /// between them is the DHT's protocol amplification factor.
    pub fn wire_stats(&self) -> (u64, u64) {
        self.dht.wire_stats()
    }

    /// Borrow the shared wire-counter handle for long-lived sampling. The
    /// returned counters are `Arc<AtomicU64>` internally; cloning is cheap.
    pub fn wire_counters(&self) -> crate::io::WireCounters {
        self.dht.wire_counters()
    }

    /// Wait until the DHT is bootstrapped.
    pub async fn bootstrapped(&self) -> Result<(), HyperDhtError> {
        self.dht.bootstrapped().await.map_err(HyperDhtError::Dht)
    }

    /// Destroy the underlying DHT instance.
    pub async fn destroy(&self) -> Result<(), HyperDhtError> {
        self.reuse.clear();
        self.dht.destroy().await.map_err(HyperDhtError::Dht)
    }

    /// Current mode of the underlying DHT node.
    pub fn mode(&self) -> crate::rpc::DhtMode {
        self.dht.mode()
    }

    /// Watches mode changes of the underlying DHT node.
    pub fn watch_mode(&self) -> tokio::sync::watch::Receiver<crate::rpc::DhtMode> {
        self.dht.watch_mode()
    }

    /// Turns adaptive mode on or off on the underlying DHT node.
    pub async fn set_adaptive(&self, on: bool) -> Result<(), HyperDhtError> {
        self.dht.set_adaptive(on).await.map_err(HyperDhtError::Dht)
    }

    /// Returns the number of nodes in the routing table.
    pub async fn table_size(&self) -> Result<usize, HyperDhtError> {
        self.dht.table_size().await.map_err(HyperDhtError::Dht)
    }

    /// Returns the routing table as host/port pairs, most recently seen first.
    pub async fn to_array(
        &self,
        limit: Option<usize>,
    ) -> Result<Vec<crate::messages::Ipv4Peer>, HyperDhtError> {
        self.dht.to_array(limit).await.map_err(HyperDhtError::Dht)
    }

    /// Returns the local port the DHT server socket is bound to.
    pub async fn local_port(&self) -> Result<u16, HyperDhtError> {
        self.dht.local_port().await.map_err(HyperDhtError::Dht)
    }

    /// Returns the DHT server socket for multiplexing UDX streams.
    pub async fn server_socket(&self) -> Result<Option<UdxSocket>, HyperDhtError> {
        self.dht.server_socket().await.map_err(HyperDhtError::Dht)
    }

    /// Returns the actual listen socket (bound to the advertised server port).
    pub async fn listen_socket(&self) -> Result<Option<UdxSocket>, HyperDhtError> {
        self.dht.listen_socket().await.map_err(HyperDhtError::Dht)
    }

    /// Returns the externally-observed IPv4 address of this node, as inferred
    /// from the `to` field of the last 16 DHT responses (dht-rpc `NatSampler`
    /// window; one sample per responder per minute).
    ///
    /// `None` until 3 samples, while the window disagrees (e.g. right after a
    /// network change), and always when running unfirewalled. May go back from
    /// `Some` to `None` and to a new address.
    pub async fn remote_address(&self) -> Result<Option<crate::messages::Ipv4Peer>, HyperDhtError> {
        self.dht.remote_address().await.map_err(HyperDhtError::Dht)
    }

    /// Build the IPv4 `addresses4` list to advertise in a noise handshake
    /// payload. Mirrors Node `hyperdht/lib/connect.js:420-437` and
    /// `hyperdht/lib/server.js:277-284`: the reflexive address from
    /// [`Self::remote_address`] (if any) is prepended, then the local
    /// IPv4 interface addresses from
    /// [`Holepuncher::local_addresses`](crate::holepuncher::Holepuncher::local_addresses)
    /// are appended. Order matters: the receiver iterates this list
    /// sequentially and LAN-special-cases on its side.
    pub async fn noise_addresses4(&self, port: u16) -> Vec<crate::messages::Ipv4Peer> {
        let mut out = Vec::new();
        if let Ok(Some(remote)) = self.remote_address().await {
            out.push(remote);
        }
        out.extend(crate::holepuncher::Holepuncher::local_addresses(port));
        out
    }

    /// Returns the current firewall classification (one of the
    /// `FIREWALL_OPEN` / `FIREWALL_UNKNOWN` / `FIREWALL_CONSISTENT` /
    /// `FIREWALL_RANDOM` constants from [`crate::hyperdht_messages`]).
    ///
    /// Mirrors Node Hyperswarm's `dht.firewalled`.
    pub async fn firewalled(&self) -> Result<u64, HyperDhtError> {
        self.dht.firewalled().await.map_err(HyperDhtError::Dht)
    }

    /// Returns the set of DHT-relay endpoints this node is currently
    /// announcing through (capped at 3, refreshed on each `announce()`).
    ///
    /// These are the addresses of DHT nodes that successfully accepted this
    /// node's last announce — meaning they hold a `ForwardEntry` and will
    /// relay `PEER_HANDSHAKE` on this node's behalf. Empty until the first
    /// `announce()` completes.
    ///
    /// Mirrors Node Hyperswarm's `Server.relayAddresses` / `Announcer.relayAddresses`.
    pub fn current_relay_addresses(&self) -> Vec<crate::messages::Ipv4Peer> {
        self.current_relay_addresses
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default()
    }

    /// Access the shared router state.
    pub fn router(&self) -> &Arc<Mutex<Router>> {
        &self.router
    }

    /// Access the underlying DHT handle.
    pub fn dht(&self) -> &DhtHandle {
        &self.dht
    }

    /// Returns persistent storage statistics collected from the request handler.
    pub async fn persistent_stats(&self) -> Result<PersistentStats, HyperDhtError> {
        let (tx, rx) = oneshot::channel();
        self.admin_tx
            .send(AdminRequest::PersistentStats { reply: tx })
            .map_err(|_| HyperDhtError::Destroyed)?;
        rx.await.map_err(|_| HyperDhtError::Destroyed)
    }

    /// Mark a target as having a local server available.
    pub fn register_server(&self, target: &[u8; 32]) {
        if let Ok(mut router) = self.router.lock() {
            router.set(
                target,
                ForwardEntry {
                    relay: None,
                    has_server: true,
                    inserted: std::time::Instant::now(),
                },
            );
        }
    }

    /// Remove the local-server marker for a target.
    pub fn unregister_server(&self, target: &[u8; 32]) {
        if let Ok(mut router) = self.router.lock() {
            router.delete(target);
        }
    }

    /// Access the server event sender.
    pub fn server_sender(&self) -> &mpsc::UnboundedSender<ServerEvent> {
        &self.server_tx
    }
}
