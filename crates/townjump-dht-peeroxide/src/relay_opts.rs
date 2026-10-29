// JS: none directly (blind-relay `RelayServer` has no options here); the reachability check is the dht-rpc
// adaptive NAT round (PING_NAT) of the fork. Options of a relay node and their translation into fork configs.
// See also: relay.rs (the relay node), opts_map.rs (connect/server options).
use peeroxide_dht::hyperdht::HyperDhtConfig;
use peeroxide_dht::hyperdht_messages::{FIREWALL_CONSISTENT, FIREWALL_OPEN, FIREWALL_RANDOM};
use peeroxide_dht::messages::Ipv4Peer;
use peeroxide_dht::relay_service::RelayServiceConfig;
use peeroxide_dht::server::Firewall as ForkFirewall;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Arc;
use townjump_dht::Firewall;
use townjump_keys::KeyPair;

/// Ticks (5 s each) before the first reachability round of a checked relay: about 10 s after bootstrap.
pub(crate) const RELAY_STABLE_TICKS: u64 = 2;

/// Options of a relay node; `Default` keeps the old behaviour (random key, everyone admitted, self-announce,
/// random port, assumed open).
#[derive(Clone)]
pub struct RelayOpts {
    /// Relay key pair; `None` — a random one. A saved pair keeps the relay key across restarts.
    pub identity: Option<KeyPair>,
    /// Admission by the peer's public key (the node key of the client or server); `true` refuses, silently.
    pub firewall: Option<Firewall>,
    /// Announce the relay key on the DHT so peers find it by key; `false` — reachable only by address.
    pub announce: bool,
    /// UDP port of the relay node; `0` — any free port. A busy port is an error at start.
    pub port: u16,
    /// `true` — the node starts persistent and open, no check is made (the old behaviour);
    /// `false` — it starts firewalled and proves reachability itself (about 10-15 s), see `reachability`.
    pub assume_open: bool,
}

impl Default for RelayOpts {
    fn default() -> Self {
        RelayOpts {
            identity: None,
            firewall: None,
            announce: true,
            port: 0,
            assume_open: true,
        }
    }
}

/// NAT type of a node, from the fork `FIREWALL_*` constants; any unknown value is `Unknown`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NatKind {
    Unknown,
    Open,
    Consistent,
    Random,
}

impl From<u64> for NatKind {
    fn from(value: u64) -> Self {
        match value {
            FIREWALL_OPEN => NatKind::Open,
            FIREWALL_CONSISTENT => NatKind::Consistent,
            FIREWALL_RANDOM => NatKind::Random,
            _ => NatKind::Unknown,
        }
    }
}

/// What the relay knows about its own reachability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelayReachability {
    /// Port of the relay's server socket.
    pub local_port: u16,
    /// Address to publish; `Some` only when `firewall == Open` and the address is known (see `publish_address`).
    pub public_address: Option<SocketAddrV4>,
    pub firewall: NatKind,
}

/// Fork node config of a relay: `base` from the factory plus port, persistence and firewall state.
pub(crate) fn relay_node_config(mut base: HyperDhtConfig, opts: &RelayOpts) -> HyperDhtConfig {
    base.dht.port = opts.port;
    if opts.assume_open {
        base.dht.ephemeral = Some(false);
        base.dht.firewalled = false;
    } else {
        // The fork runs the NAT round only for an adaptive node with an undecided `ephemeral`.
        base.dht.ephemeral = None;
        base.dht.adaptive = true;
        base.dht.firewalled = true;
        base.dht.stable_ticks = RELAY_STABLE_TICKS;
    }
    base
}

/// Relay service config: defaults plus self-announce and admission (the fork type is `#[non_exhaustive]`).
pub(crate) fn service_config(opts: &RelayOpts) -> RelayServiceConfig {
    let mut config = RelayServiceConfig::default();
    config.announce = opts.announce;
    config.peer_firewall = opts.firewall.clone().map(fork_firewall);
    config
}

/// holesail firewall (sees the public key) → fork firewall (sees the whole handshake input).
pub(crate) fn fork_firewall(f: Firewall) -> ForkFirewall {
    Arc::new(move |input| f(input.remote_public_key))
}

/// Address worth publishing: only for an open node; the declared (proven) address first, then the observed one.
/// The observed address of a firewalled node is never used: its port belongs to the client socket.
pub(crate) fn publish_address(
    firewall: NatKind,
    declared: Option<Ipv4Peer>,
    observed: Option<Ipv4Peer>,
) -> Option<SocketAddrV4> {
    if firewall != NatKind::Open {
        return None;
    }
    let peer = declared.or(observed)?;
    let host: Ipv4Addr = peer.host.parse().ok()?;
    if host.is_unspecified() || peer.port == 0 {
        return None;
    }
    Some(SocketAddrV4::new(host, peer.port))
}

#[cfg(test)]
#[path = "relay_opts_tests.rs"]
mod relay_opts_tests;
