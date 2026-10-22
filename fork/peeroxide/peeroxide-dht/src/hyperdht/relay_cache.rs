// JS: hyperdht/index.js:49,624 (`_relayAddressesCache`), lib/connect.js:353-377,508
//! Relay addresses that served the last connect to a target, keyed by the target's public key.
//! Neighbours: `connect.rs` reads it as `pre`, `connect_finish.rs` writes it after a handshake.

use std::collections::{HashMap, VecDeque};

use crate::messages::Ipv4Peer;

/// Entries kept; JS `index.js:49` caps its cache at the same 512, without an expiry.
const CAPACITY: usize = 512;

#[derive(Default)]
pub(super) struct RelayCache {
    map: HashMap<[u8; 32], Vec<Ipv4Peer>>,
    /// Keys from the oldest write to the newest.
    order: VecDeque<[u8; 32]>,
}

impl RelayCache {
    /// Addresses stored for `key`, empty when there are none.
    pub(super) fn get(&self, key: &[u8; 32]) -> Vec<Ipv4Peer> {
        self.map.get(key).cloned().unwrap_or_default()
    }

    /// Replaces the entry of `key`; a full cache drops its oldest entry first.
    pub(super) fn set(&mut self, key: [u8; 32], relays: Vec<Ipv4Peer>) {
        if self.map.insert(key, relays).is_some() {
            self.order.retain(|k| k != &key);
        } else if self.map.len() > CAPACITY
            && let Some(oldest) = self.order.pop_front()
        {
            self.map.remove(&oldest);
        }
        self.order.push_back(key);
    }
}

#[cfg(test)]
#[path = "relay_cache_tests.rs"]
mod tests;
