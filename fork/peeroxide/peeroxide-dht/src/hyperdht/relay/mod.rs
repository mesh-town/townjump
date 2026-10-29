//! Client side of blind-relay use: relay choice and token, the role in a pair, shared small types.
// JS: hyperdht/lib/connect.js:498-536

mod link;
mod pair;
mod select;
#[cfg(test)]
mod test_support;
mod upgrade;

use std::fmt;

use tokio::task::JoinHandle;

use crate::hyperdht_messages::RelayThroughInfo;
use crate::messages::Ipv4Peer;

pub(crate) use link::RelayLink;
pub(crate) use pair::{RelayPaired, pair_via_relay};
pub(crate) use select::gate_punch;
pub use select::{HolepunchFilter, HolepunchInput, RelayThrough};
pub(crate) use upgrade::{DirectSource, Upgrade};

/// Which path a relayed connection currently uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RelayPath {
    /// Data still flows through the relay.
    Relayed,
    /// The connection moved to the direct path.
    Direct,
}

/// A relay identified by its public key together with a known network address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayAddress {
    /// Public key of the relay.
    pub public_key: [u8; 32],
    /// Address the relay can be reached at.
    pub addr: Ipv4Peer,
}

/// Addresses listed for `key`, in the original order; empty when there are none.
pub(crate) fn addresses_for(list: &[RelayAddress], key: &[u8; 32]) -> Vec<Ipv4Peer> {
    list.iter()
        .filter(|a| &a.public_key == key)
        .map(|a| a.addr.clone())
        .collect()
}

/// A relay this client asked for, with the pairing token (kept secret).
#[derive(Clone)]
pub(crate) struct RelayAsk {
    pub public_key: [u8; 32],
    pub token: [u8; 32],
}

impl fmt::Debug for RelayAsk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RelayAsk")
            .field("public_key", &self.public_key)
            .field("token", &"..")
            .finish()
    }
}

impl RelayAsk {
    /// The same relay with a token of its own, for one handshake of a connect: concurrent
    /// handshakes land in separate server slots and only the completed one may be paired.
    pub(crate) fn for_handshake(&self) -> RelayAsk {
        RelayAsk {
            public_key: self.public_key,
            token: rand::random::<[u8; 32]>(),
        }
    }
}

/// Choose a relay and a fresh token; `None` when no relay is wanted.
pub(crate) fn resolve(rt: Option<&RelayThrough>) -> Option<RelayAsk> {
    let public_key = rt?.select()?;
    Some(RelayAsk {
        public_key,
        token: rand::random::<[u8; 32]>(),
    })
}

/// The relay and role the client uses for pairing.
pub(crate) struct RelayRole {
    pub public_key: [u8; 32],
    pub token: [u8; 32],
    pub is_initiator: bool,
}

/// Server's `relay_through` wins (we are not the initiator); otherwise our own ask (initiator).
pub(crate) fn client_role(
    remote: Option<&RelayThroughInfo>,
    ours: Option<&RelayAsk>,
) -> Option<RelayRole> {
    if let Some(r) = remote {
        return Some(RelayRole {
            public_key: r.public_key,
            token: r.token,
            is_initiator: false,
        });
    }
    let a = ours?;
    Some(RelayRole {
        public_key: a.public_key,
        token: a.token,
        is_initiator: true,
    })
}

/// A task handle that aborts the task when dropped.
pub(crate) struct AbortOnDrop<T>(pub JoinHandle<T>);

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[cfg(test)]
#[path = "select_tests.rs"]
mod tests;
