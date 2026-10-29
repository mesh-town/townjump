//! Standalone entry point that runs a blind-relay server on top of a
//! [`crate::hyperdht::HyperDhtHandle`].
//!
//! This module wires the protocol-only engine in [`crate::blind_relay`] to
//! real transport: it accepts incoming `PEER_HANDSHAKE` requests (via
//! [`crate::hyperdht::ServerEvent`]), finalizes each into an encrypted
//! connection, opens a `"blind-relay"` Protomux channel on it, and drives a
//! [`crate::blind_relay::BlindRelaySession`] against a shared
//! [`crate::blind_relay::BlindRelayServer`]. When a pairing matches, it
//! creates the two raw UDX data-plane streams and bridges them with
//! [`libudx::UdxStream::relay_to`] (packet-level, blind — peeroxide never
//! decrypts relayed application data).
//!
//! `run_relay_server` also self-announces `hash(public_key)` through
//! [`crate::hyperdht::HyperDhtHandle::announce`] at startup and then on a
//! roughly 10-minute refresh loop, mirroring Node's `Server.listen()`
//! announcer (`RelayServiceConfig::announce = false` skips it; the relay then
//! answers only handshakes sent straight to its address). That announce record is what Node.js `hyperdht` clients discover
//! during `dht.connect()`/`findPeer`; `register_server` alone only makes the
//! local node *answer* handshakes once they arrive.
//!
//! The service additionally runs periodic maintenance around the shared
//! pairing table: it sweeps expired pending pairings, closes sessions idle past
//! `idle_session_timeout`, and tears down bridged data-plane streams when an
//! active token is unpaired or either session closes. See
//! [`crate::blind_relay`] for the protocol-level pairing/session lifecycle.
//!
//! Deliberately independent of upstream `peeroxide::Swarm`: a relay has no topics,
//! no peer discovery, and no retry bookkeeping, so this reimplements only
//! the minimal "accept a handshake, finalize a connection" path rather than
//! reusing `Swarm`'s internals.
//!
//! Holepunching is intentionally out of scope for v1: a relay is expected
//! to run with a directly reachable address (matches how
//! `blind-relay-service`'s reference deployment works — a plain
//! `dht.createServer()` with no NAT-traversal staging on the relay's own
//! control connections).
//!
//! Admission: `RelayServiceConfig::peer_firewall` sees each connecting peer's
//! key; a refused peer gets no reply and no session (like a server firewall).
//!
//! Stop: every task of the service (accept loop, self-announce, sweep, bridge,
//! sessions and their Mux drivers) is owned by the returned
//! [`RelayServiceHandle`]; `close` stops them and withdraws the relay,
//! dropping the handle aborts them.
//!
//! Files: `handshake` (one PEER_HANDSHAKE, admission), `dedup` (repeated handshakes), `session` (control session),
//! `bridge` (data plane), `stop` (task ownership and the stop handle).
// JS: blind-relay/index.js

mod bridge;
mod dedup;
mod handshake;
mod session;
mod stop;
#[cfg(test)]
mod test_net;

use std::sync::Arc;
use std::time::Duration;

use libudx::RuntimeHandle;
use rand::Rng;
use tokio::sync::mpsc;

use crate::blind_relay::{BlindRelayServer, BlindRelayServerConfig, MatchedPairing};
use crate::hyperdht::{HyperDhtHandle, KeyPair, ServerEvent};
use crate::hyperdht_messages::FIREWALL_UNKNOWN;
use crate::server::Firewall;

use bridge::bridge_matched_pairings;
use dedup::Dedup;
use handshake::handle_handshake;
use stop::TaskSet;

pub use stop::RelayServiceHandle;

/// Configuration for [`run_relay_server`].
#[derive(Clone)]
#[non_exhaustive]
pub struct RelayServiceConfig {
    /// Limits/timeouts for the shared [`BlindRelayServer`] engine.
    pub relay: BlindRelayServerConfig,
    /// How often to sweep expired (timed-out) pending pairings.
    pub sweep_interval: Duration,
    /// Firewall state advertised to connecting peers in the noise reply.
    /// A relay is expected to run reachably, so this defaults to
    /// `FIREWALL_UNKNOWN` (matches Node's default — the relay doesn't
    /// participate in NAT classification for its own control connections).
    pub firewall: u64,
    /// Announce `hash(public_key)` on the DHT at startup and on refresh (default `true`).
    pub announce: bool,
    /// Admission callback for connecting peers; `true` refuses (silence, no reply). `None` admits all.
    pub peer_firewall: Option<crate::server::Firewall>,
}

impl Default for RelayServiceConfig {
    fn default() -> Self {
        Self {
            relay: BlindRelayServerConfig::default(),
            sweep_interval: Duration::from_secs(30),
            firewall: FIREWALL_UNKNOWN,
            announce: true,
            peer_firewall: None,
        }
    }
}

impl std::fmt::Debug for RelayServiceConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RelayServiceConfig")
            .field("relay", &self.relay)
            .field("sweep_interval", &self.sweep_interval)
            .field("firewall", &self.firewall)
            .field("announce", &self.announce)
            .field(
                "peer_firewall",
                &self.peer_firewall.as_ref().map(|_| "<callback>"),
            )
            .finish()
    }
}

/// Everything a handshake needs from the running service (cheap to clone).
#[derive(Clone)]
struct Service {
    runtime_handle: Arc<RuntimeHandle>,
    dht: HyperDhtHandle,
    key_pair: KeyPair,
    /// Firewall state advertised in the noise reply.
    firewall: u64,
    /// Admission callback; `None` admits all.
    peer_firewall: Option<Firewall>,
    relay: BlindRelayServer,
    matched_tx: mpsc::UnboundedSender<MatchedPairing>,
    tasks: TaskSet,
}

/// Errors from running the relay service.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RelayServiceError {
    /// The DHT server-event channel closed unexpectedly.
    #[error("DHT server-event channel closed")]
    ChannelClosed,
}

/// Run a blind-relay server against an already-spawned [`HyperDhtHandle`].
///
/// Consumes `server_rx` (the `ServerEvent` receiver returned by
/// `hyperdht::spawn`) until it closes or the service is stopped. Returns the
/// shared [`BlindRelayServer`] handle (for stats/inspection — e.g. periodic
/// logging) alongside the [`RelayServiceHandle`] that owns every task of the
/// service: `close` withdraws it, dropping the handle aborts it.
pub fn run_relay_server(
    runtime_handle: Arc<RuntimeHandle>,
    dht: HyperDhtHandle,
    key_pair: KeyPair,
    server_rx: mpsc::UnboundedReceiver<ServerEvent>,
    config: RelayServiceConfig,
) -> (BlindRelayServer, RelayServiceHandle) {
    // Register our own identity so the DHT layer's handshake router treats
    // inbound `PEER_HANDSHAKE` requests targeting `hash(public_key)` as
    // "handle locally" instead of replying CLOSER_NODES (mirrors
    // upstream `peeroxide::swarm`'s `do_join(server: true)` — without this, nothing
    // ever reaches this module's handshake handler at all). Kept without the
    // self-announce too: a handshake sent straight to the relay's address is
    // still answered.
    let target = crate::crypto::hash(&key_pair.public_key);
    dht.register_server(&target);

    let tasks = TaskSet::new();
    if config.announce {
        tasks.spawn(announce_loop(dht.clone(), key_pair.clone(), target));
    }

    let relay = BlindRelayServer::new(config.relay.clone());
    let (matched_tx, matched_rx) = mpsc::unbounded_channel::<MatchedPairing>();

    // Wires the raw UDX data-plane streams for every matched pairing and
    // bridges them bidirectionally.
    tasks.spawn(bridge_matched_pairings(
        Arc::clone(&runtime_handle),
        dht.clone(),
        relay.clone(),
        matched_rx,
        tasks.clone(),
    ));
    tasks.spawn(sweep_loop(relay.clone(), config.sweep_interval));

    let service = Service {
        runtime_handle,
        dht: dht.clone(),
        key_pair: key_pair.clone(),
        firewall: config.firewall,
        peer_firewall: config.peer_firewall,
        relay: relay.clone(),
        matched_tx,
        tasks: tasks.clone(),
    };
    tasks.spawn(accept_loop(service, server_rx));

    let handle = RelayServiceHandle::new(tasks, dht, key_pair, target, config.announce);
    (relay, handle)
}

/// Self-announce `hash(public_key)` at startup and on a ~10-minute refresh.
///
/// Matches Node's `Server.listen()`, which starts an `Announcer` for the
/// server's own target. Without this, `register_server` only makes *this*
/// node answer PEER_HANDSHAKE locally once a request arrives — but nothing
/// tells the rest of the network we exist, so a remote client's
/// `dht.connect(pubkey)`/`findPeer` walk (a LOOKUP-style query for an
/// announced record on `hash(pubkey)`) never surfaces us and fails with
/// PEER_NOT_FOUND. Mirrors upstream `peeroxide::peer_discovery::do_refresh`'s
/// "self_announce" pattern.
async fn announce_loop(dht: HyperDhtHandle, key_pair: KeyPair, target: [u8; 32]) {
    const REFRESH_INTERVAL: Duration = Duration::from_secs(600);
    const REFRESH_JITTER_MS: u64 = 120_000;

    loop {
        let relays = dht.current_relay_addresses();
        match dht.announce(target, &key_pair, &relays).await {
            Ok(r) => {
                tracing::debug!(
                    closest = r.closest_nodes.len(),
                    "relay: self-announce complete"
                );
            }
            Err(e) => {
                tracing::warn!(err = %e, "relay: self-announce failed");
            }
        }
        let jitter_ms = rand::rng().random_range(0..REFRESH_JITTER_MS);
        tokio::time::sleep(REFRESH_INTERVAL + Duration::from_millis(jitter_ms)).await;
    }
}

/// Periodically sweep pending pairings that timed out and sessions idle past
/// `idle_session_timeout` (peeroxide-only — see
/// `BlindRelayServerConfig::idle_session_timeout`).
async fn sweep_loop(relay: BlindRelayServer, interval: Duration) {
    let mut ticker = tokio::time::interval(interval);
    ticker.tick().await; // skip immediate first tick
    loop {
        ticker.tick().await;
        relay.sweep_expired_pairings();
        relay.sweep_idle_sessions();
    }
}

/// Answer server events until the node's channel closes; then the service is
/// useless without its node, so every task is aborted.
async fn accept_loop(service: Service, mut server_rx: mpsc::UnboundedReceiver<ServerEvent>) {
    let mut dedup = Dedup::default();
    while let Some(event) = server_rx.recv().await {
        match event {
            ServerEvent::PeerHandshake {
                msg,
                from,
                peer_address,
                reply_tx,
                ..
            } => {
                handle_handshake(&service, &mut dedup, msg, from, peer_address, reply_tx).await;
            }
            ServerEvent::PeerHolepunch { reply_tx, .. } => {
                // Not supported in v1 — see module docs. Politely decline.
                let _ = reply_tx.send(None);
            }
        }
    }
    service.tasks.abort_now();
}
