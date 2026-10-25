//! `ServerConfig`, `ConnectOpts`, `HyperDhtConfig` and the public bootstrap list.

use crate::persistent::PersistentConfig;
use std::time::Duration;

use crate::rpc::DhtConfig;

#[cfg(test)]
use super::connect_lan::LanRewrite;
#[cfg(doc)]
use super::handle::HyperDhtHandle;
use super::keypair::KeyPair;
use super::relay::{HolepunchFilter, RelayAsk, RelayThrough};

/// Configuration used by the server-side handshake and holepunch handler.
#[non_exhaustive]
pub struct ServerConfig {
    /// Server identity key pair.
    pub key_pair: KeyPair,
    /// Firewall mode advertised to connecting peers.
    pub firewall: u64,
}

impl ServerConfig {
    /// Create a new server configuration.
    pub fn new(key_pair: KeyPair, firewall: u64) -> Self {
        Self { key_pair, firewall }
    }
}

// ── Bootstrap defaults ────────────────────────────────────────────────────────

/// The three public HyperDHT bootstrap nodes (from `hyperdht/lib/constants.js`).
///
/// Format: `suggestedIP@hostname:port`.  `parse_bootstrap_str`
/// extracts the IP before `@`, so these work without DNS resolution.
pub const DEFAULT_BOOTSTRAP: [&str; 3] = [
    "88.99.3.86@node1.hyperdht.org:49737",
    "142.93.90.113@node2.hyperdht.org:49737",
    "138.68.147.8@node3.hyperdht.org:49737",
];

// ── Config ────────────────────────────────────────────────────────────────────

/// Per-connect options for [`HyperDhtHandle::connect_with_options`].
///
/// Default: same-NAT LAN-shortcut enabled (matches Node hyperdht's
/// `opts.localConnection` default of `true`).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ConnectOpts {
    /// Allow the same-NAT LAN-shortcut. When `false`, the receiver
    /// ignores any loopback/private addresses the server advertised in
    /// its `addresses4` and dials only the public address, and the client does
    /// not announce its own local addresses in the handshake. Set this to
    /// `false` to force the real-network code path under test.
    pub local_connection: bool,
    /// Retry over the route (socket + server address) of an earlier connection to the same
    /// server before FIND_PEER. JS `opts.reusableSocket`, default `false` (`connect.js:80`).
    pub reusable_socket: bool,
    /// Relay(s) to ask for a blind-relay pairing next to the direct attempt. JS `opts.relayThrough`.
    pub relay_through: Option<RelayThrough>,
    /// Keep-alive of the connection to the relay; `Duration::ZERO` leaves it off.
    /// JS `opts.relayKeepAlive`, default 5000 ms (`connect.js:106`).
    pub relay_keep_alive: Duration,
    /// Veto on holepunching, consulted once the peer answered the first probe. JS `opts.holepunch`.
    pub holepunch: Option<HolepunchFilter>,
    /// Test lever: wait this long before the client starts holepunching (only with a relay).
    #[doc(hidden)]
    pub punch_delay: Option<Duration>,
    /// Relay chosen by `connect_with_options` for this connect; internal.
    pub(crate) relay_ask: Option<RelayAsk>,
    /// Test lever: replaces the private candidates probed next to hole-punching.
    #[cfg(test)]
    pub(crate) lan_rewrite: Option<LanRewrite>,
}

impl Default for ConnectOpts {
    fn default() -> Self {
        Self {
            local_connection: true,
            reusable_socket: false,
            relay_through: None,
            relay_keep_alive: Duration::from_millis(5000),
            holepunch: None,
            punch_delay: None,
            relay_ask: None,
            #[cfg(test)]
            lan_rewrite: None,
        }
    }
}

#[derive(Debug, Clone, Default)]
/// Configuration for a HyperDHT instance.
#[non_exhaustive]
pub struct HyperDhtConfig {
    /// DHT transport and bootstrap settings.
    pub dht: DhtConfig,
    /// Persistent storage settings for stored records.
    pub persistent: PersistentConfig,
}

impl HyperDhtConfig {
    /// Create a config pre-populated with the public HyperDHT bootstrap nodes.
    ///
    /// This is the typical starting point for connecting to the live network.
    /// `DhtConfig::default()` intentionally keeps `bootstrap` empty so that
    /// unit tests can run without network access.
    pub fn with_public_bootstrap() -> Self {
        Self {
            dht: DhtConfig {
                bootstrap: DEFAULT_BOOTSTRAP.iter().map(|s| (*s).to_string()).collect(),
                ..DhtConfig::default()
            },
            persistent: PersistentConfig::default(),
        }
    }
}
