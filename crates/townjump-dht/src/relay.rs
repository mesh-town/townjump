// JS: hyperdht `relayThrough`, `relayAddresses`-style relay hints, `holepunch` filter, and the connection path label.
// Related: types.rs (these types appear in `ConnectOpts`/`ServerOpts`/`DhtConnection`).
use std::net::SocketAddrV4;
use std::sync::Arc;

/// Which blind-relay node(s) to pair through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelayThrough {
    /// One fixed relay, by public key.
    Key([u8; 32]),
    /// Several candidates; the client tries them one by one.
    List(Vec<[u8; 32]>),
}

impl From<[u8; 32]> for RelayThrough {
    fn from(key: [u8; 32]) -> Self {
        RelayThrough::Key(key)
    }
}

/// A relay public key together with an address it can be reached at (no DHT lookup needed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RelayAddress {
    pub public_key: [u8; 32],
    pub addr: SocketAddrV4,
}

/// Whether the data of a connection currently flows through a relay or directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayPath {
    Relayed,
    Direct,
}

/// What the holepunch filter sees once the peer has answered the first probe.
#[derive(Debug, Clone)]
pub struct HolepunchInfo {
    pub remote_firewall: u64,
    pub local_firewall: u64,
    pub remote_addresses: Vec<SocketAddrV4>,
    pub local_addresses: Vec<SocketAddrV4>,
}

/// Holepunch veto: `true` = the punch may go on.
pub type HolepunchFilter = Arc<dyn Fn(&HolepunchInfo) -> bool + Send + Sync>;
