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
//! announcer. That announce record is what Node.js `hyperdht` clients discover
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
//! Files: `handshake` (one PEER_HANDSHAKE), `dedup` (repeated handshakes), `session` (control session),
//! `bridge` (data plane).
// JS: blind-relay/index.js

mod bridge;
mod dedup;
mod handshake;
mod session;

use std::sync::Arc;
use std::time::Duration;

use libudx::RuntimeHandle;
use rand::Rng;
use tokio::sync::mpsc;

use crate::blind_relay::{BlindRelayServer, BlindRelayServerConfig, MatchedPairing};
use crate::hyperdht::{HyperDhtHandle, KeyPair, ServerEvent};
use crate::hyperdht_messages::FIREWALL_UNKNOWN;

use bridge::bridge_matched_pairings;
use dedup::Dedup;
use handshake::handle_handshake;

/// Configuration for [`run_relay_server`].
#[derive(Debug, Clone)]
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
}

impl Default for RelayServiceConfig {
    fn default() -> Self {
        Self {
            relay: BlindRelayServerConfig::default(),
            sweep_interval: Duration::from_secs(30),
            firewall: FIREWALL_UNKNOWN,
        }
    }
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
/// `hyperdht::spawn`) until it closes or the returned `shutdown_rx` fires.
/// Returns the shared [`BlindRelayServer`] handle (for stats/inspection —
/// e.g. periodic logging) alongside the driving task.
pub fn run_relay_server(
    runtime_handle: Arc<RuntimeHandle>,
    dht: HyperDhtHandle,
    key_pair: KeyPair,
    mut server_rx: mpsc::UnboundedReceiver<ServerEvent>,
    config: RelayServiceConfig,
) -> (BlindRelayServer, tokio::task::JoinHandle<()>) {
    // Register our own identity so the DHT layer's handshake router treats
    // inbound `PEER_HANDSHAKE` requests targeting `hash(public_key)` as
    // "handle locally" instead of replying CLOSER_NODES (mirrors
    // upstream `peeroxide::swarm`'s `do_join(server: true)` — without this, nothing
    // ever reaches this module's handshake handler at all).
    let self_target = crate::crypto::hash(&key_pair.public_key);
    dht.register_server(&self_target);

    // Self-announce our identity (`hash(public_key)`) to the DHT, matching
    // Node's `Server.listen()` which internally starts an `Announcer` for
    // the server's own target. Without this, `register_server` only makes
    // *this* node answer PEER_HANDSHAKE locally once a request arrives —
    // but nothing tells the rest of the network we exist, so a remote
    // client's `dht.connect(pubkey)`/`findPeer` walk (which is a
    // LOOKUP-style query for an announced record on `hash(pubkey)`, not a
    // raw closest-node walk) never surfaces us as a candidate at all and
    // fails with PEER_NOT_FOUND, as with Node's
    // reference `hyperdht` client. Mirrors
    // upstream `peeroxide::peer_discovery::do_refresh`'s "self_announce" pattern.
    let announce_dht = dht.clone();
    let announce_key_pair = key_pair.clone();
    tokio::spawn(async move {
        const REFRESH_INTERVAL: Duration = Duration::from_secs(600);
        const REFRESH_JITTER_MS: u64 = 120_000;

        loop {
            let relays = announce_dht.current_relay_addresses();
            match announce_dht
                .announce(self_target, &announce_key_pair, &relays)
                .await
            {
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
    });

    let relay = BlindRelayServer::new(config.relay.clone());
    let relay_for_task = relay.clone();

    let (matched_tx, matched_rx) = mpsc::unbounded_channel::<MatchedPairing>();

    // Background task: wires the raw UDX data-plane streams for every
    // matched pairing and bridges them bidirectionally.
    let bridge_runtime_handle = Arc::clone(&runtime_handle);
    let bridge_dht = dht.clone();
    let bridge_relay = relay.clone();
    tokio::spawn(bridge_matched_pairings(
        bridge_runtime_handle,
        bridge_dht,
        bridge_relay,
        matched_rx,
    ));

    // Background task: periodically sweep pending pairings that timed out
    // and sessions idle past `idle_session_timeout` (peeroxide-only — see
    // BlindRelayServerConfig::idle_session_timeout).
    let sweep_relay = relay.clone();
    let sweep_interval = config.sweep_interval;
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(sweep_interval);
        ticker.tick().await; // skip immediate first tick
        loop {
            ticker.tick().await;
            sweep_relay.sweep_expired_pairings();
            sweep_relay.sweep_idle_sessions();
        }
    });

    let firewall = config.firewall;
    let task = tokio::spawn(async move {
        let mut dedup = Dedup::default();
        while let Some(event) = server_rx.recv().await {
            match event {
                ServerEvent::PeerHandshake {
                    msg,
                    from: _,
                    peer_address,
                    target: _,
                    reply_tx,
                    ..
                } => {
                    handle_handshake(
                        &runtime_handle,
                        &dht,
                        &key_pair,
                        firewall,
                        &relay_for_task,
                        &matched_tx,
                        &mut dedup,
                        msg,
                        peer_address,
                        reply_tx,
                    )
                    .await;
                }
                ServerEvent::PeerHolepunch { reply_tx, .. } => {
                    // Not supported in v1 — see module docs. Politely decline.
                    let _ = reply_tx.send(None);
                }
            }
        }
    });

    (relay, task)
}
