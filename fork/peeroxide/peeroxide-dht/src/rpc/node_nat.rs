// JS: dht-rpc/index.js (_addNodeFromNetwork sampling, remoteAddress)
//! Rust extension: per-responder resample gate (`RECENT_NODE` ticks, JS uses table nodes and
//! `OLD_NODE`) and an address for firewalled nodes too (JS `remoteAddress()` is null there;
//! `same_host` relies on it).

use super::consts::RECENT_NODE;
use super::nat_window::NatWindow;
use crate::hyperdht_messages::{
    FIREWALL_CONSISTENT, FIREWALL_OPEN, FIREWALL_RANDOM, FIREWALL_UNKNOWN,
};
use crate::messages::Ipv4Peer;
use std::collections::{HashMap, HashSet};

/// Samples required before an address is reported (as `Nat` in `nat.rs` does).
pub(super) const MIN_SAMPLES: u32 = 3;
const RESAMPLE_TICKS: u64 = RECENT_NODE;
const PRUNE_AT: usize = 256;

pub(super) struct NodeNat {
    window: NatWindow,
    last_sampled: HashMap<(String, u16), u64>,
    firewalled: bool,
    /// Server port this node was verified reachable on; set by `open`.
    public_port: Option<u16>,
    /// Accepted responders; grows only while it holds fewer than `MIN_SAMPLES` entries.
    distinct_from: HashSet<(String, u16)>,
}

impl NodeNat {
    pub(super) fn new(firewalled: bool) -> Self {
        Self {
            window: NatWindow::new(),
            last_sampled: HashMap::new(),
            firewalled,
            public_port: None,
            distinct_from: HashSet::new(),
        }
    }

    /// Feeds `to` unless it is empty (port 0 or empty host) or `from` was sampled less than
    /// `RESAMPLE_TICKS` ago. Returns whether the sample entered the window.
    pub(super) fn sample(&mut self, to: &Ipv4Peer, from: &Ipv4Peer, tick: u64) -> bool {
        if to.port == 0 || to.host.is_empty() {
            return false;
        }
        let key = (from.host.clone(), from.port);
        if let Some(&last) = self.last_sampled.get(&key)
            && tick.saturating_sub(last) < RESAMPLE_TICKS
        {
            return false;
        }
        if self.last_sampled.len() >= PRUNE_AT {
            self.last_sampled
                .retain(|_, &mut t| tick.saturating_sub(t) < RESAMPLE_TICKS);
        }
        self.last_sampled.insert(key.clone(), tick);
        // latch: a firewalled node reports an address only after 3 different responders
        // were sampled, as `Nat` in `nat.rs` does
        if self.distinct_from.len() < MIN_SAMPLES as usize {
            self.distinct_from.insert(key);
        }
        self.window.add(&to.host, to.port);
        true
    }

    /// (address, FIREWALL_*): open nodes have no address; otherwise follows the window.
    pub(super) fn remote_state(&self) -> (Option<Ipv4Peer>, u64) {
        if !self.firewalled {
            return (self.public_address(), FIREWALL_OPEN);
        }
        if self.window.size() < MIN_SAMPLES || self.distinct_from.len() < MIN_SAMPLES as usize {
            return (None, FIREWALL_UNKNOWN);
        }
        let Some(host) = self.window.host() else {
            return (None, FIREWALL_UNKNOWN);
        };
        let host = host.to_owned();
        match self.window.port() {
            0 => (Some(Ipv4Peer { host, port: 0 }), FIREWALL_RANDOM),
            port => (Some(Ipv4Peer { host, port }), FIREWALL_CONSISTENT),
        }
    }

    /// Window host even while its port is 0 (JS `_nat.host`).
    pub(super) fn host(&self) -> Option<&str> {
        self.window.host()
    }

    /// Window address; `None` for an empty host or port 0.
    pub(super) fn address(&self) -> Option<Ipv4Peer> {
        let host = self.window.host()?.to_owned();
        match self.window.port() {
            0 => None,
            port => Some(Ipv4Peer { host, port }),
        }
    }

    /// The node proved reachable on `server_port`: stop being firewalled.
    pub(super) fn open(&mut self, server_port: u16) {
        self.firewalled = false;
        self.public_port = Some(server_port);
    }

    /// Back to the configured `firewalled` state (used after a wake-up).
    pub(super) fn close(&mut self, firewalled: bool) {
        self.firewalled = firewalled;
        self.public_port = None;
    }

    pub(super) fn replace_window(&mut self, window: NatWindow) {
        self.window = window;
    }

    /// Rust extension (JS `remoteAddress` 201-215 differs): an opened node publishes the window
    /// address only when its port is the one verified reachable.
    fn public_address(&self) -> Option<Ipv4Peer> {
        let address = self.address()?;
        (self.public_port == Some(address.port)).then_some(address)
    }

    #[cfg(test)]
    pub(super) fn window(&self) -> &NatWindow {
        &self.window
    }
}

#[cfg(test)]
#[path = "node_nat_tests.rs"]
mod tests;
