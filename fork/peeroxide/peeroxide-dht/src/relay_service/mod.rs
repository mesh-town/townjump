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
//! Control connections come from a regular [`crate::server::Server`] listening on the relay's
//! key: a client behind a NAT is hole-punched (or dialed back when it is open), a peer whose
//! own NAT is random is refused a punch. The relay's self-announce stays here and hands the
//! relays that accepted it to the server for the holepunch info of its replies.
//!
//! Admission: `RelayServiceConfig::peer_firewall` sees each connecting peer's
//! key; a refused peer gets no reply and no session (like a server firewall).
//!
//! Stop: every task of the service (accept loop, self-announce, sweep, bridge,
//! sessions and their Mux drivers) is owned by the returned
//! [`RelayServiceHandle`]; `close` stops them and withdraws the relay,
//! dropping the handle aborts them.
//!
//! Files: `accept` (server options, control connections), `session` (control session and its socket),
//! `bridge` (data plane), `stop` (task ownership and the stop handle).
// JS: blind-relay/index.js

mod accept;
mod bridge;
#[cfg(test)]
#[path = "nat_tests.rs"]
mod nat_tests;
#[cfg(test)]
#[path = "relays_tests.rs"]
mod relays_tests;
mod session;
mod stop;
#[cfg(test)]
mod test_net;

use std::sync::Arc;
use std::time::Duration;

use libudx::{RuntimeHandle, UdxRuntime};
use rand::Rng;
use tokio::sync::mpsc;

use crate::blind_relay::{BlindRelayServer, BlindRelayServerConfig, MatchedPairing};
use crate::hyperdht::{HyperDhtHandle, KeyPair, ServerEvent};
use crate::server::Server;

use bridge::bridge_matched_pairings;
use session::SessionSockets;
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
            .field("announce", &self.announce)
            .field(
                "peer_firewall",
                &self.peer_firewall.as_ref().map(|_| "<callback>"),
            )
            .finish()
    }
}

/// Everything a control session needs from the running service (cheap to clone).
#[derive(Clone)]
struct Service {
    relay: BlindRelayServer,
    matched_tx: mpsc::UnboundedSender<MatchedPairing>,
    sockets: SessionSockets,
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
    let target = crate::crypto::hash(&key_pair.public_key);
    let tasks = TaskSet::new();
    let (to_server_tx, to_server_rx) = mpsc::unbounded_channel::<ServerEvent>();
    let (server, incoming) = Server::new(
        dht.clone(),
        to_server_rx,
        UdxRuntime::shared(Arc::clone(&runtime_handle)),
        accept::server_opts(&config),
    );
    if let Err(e) = server.listen_unannounced(key_pair.clone()) {
        tracing::warn!(err = %e, "relay: control-connection server failed to start");
    }
    let server = Arc::new(server);
    tasks.spawn(forward_events(server_rx, to_server_tx, tasks.clone()));
    if config.announce {
        tasks.spawn(announce_loop(
            dht.clone(),
            key_pair.clone(),
            target,
            Arc::clone(&server),
        ));
    }

    let relay = BlindRelayServer::new(config.relay.clone());
    let (matched_tx, matched_rx) = mpsc::unbounded_channel::<MatchedPairing>();

    // Wires the raw UDX data-plane streams for every matched pairing and
    // bridges them bidirectionally.
    let sockets = SessionSockets::default();
    tasks.spawn(bridge_matched_pairings(
        Arc::clone(&runtime_handle),
        relay.clone(),
        matched_rx,
        tasks.clone(),
        sockets.clone(),
    ));
    tasks.spawn(sweep_loop(relay.clone(), config.sweep_interval));

    let service = Service {
        relay: relay.clone(),
        matched_tx,
        sockets,
        tasks: tasks.clone(),
    };
    tasks.spawn(accept::accept_loop(service, incoming));

    let handle = RelayServiceHandle::new(tasks, dht, key_pair, target, config.announce, server);
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
async fn announce_loop(
    dht: HyperDhtHandle,
    key_pair: KeyPair,
    target: [u8; 32],
    server: Arc<Server>,
) {
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
                // Every node that accepted the record may carry a punch round; the
                // server verifies a client's address only through one of its relays.
                server.set_relays(r.relays.clone());
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

/// Pass the node's server events on to the control-connection server; once the node's
/// channel closes the service is useless without its node, so every task is aborted (the
/// server's own task does not notice a closed event channel).
async fn forward_events(
    mut from: mpsc::UnboundedReceiver<ServerEvent>,
    to: mpsc::UnboundedSender<ServerEvent>,
    tasks: TaskSet,
) {
    while let Some(event) = from.recv().await {
        let _ = to.send(event);
    }
    tasks.abort_now();
}
