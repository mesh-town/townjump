// JS: dht-rpc/index.js
//! One PING_NAT check round (`_updateNetworkState`): counts replies that arrived on the server
//! socket and decides whether the sampled address is trustworthy. Pure state, no sockets.

use super::nat_window::NatWindow;
use crate::messages::Ipv4Peer;
use crate::query::parse_bootstrap_str;

/// Targets asked per round.
const CHECK_TARGETS: usize = 5;

pub(super) struct NatCheck {
    pending: usize,
    asked: usize,
    replies: u32,
    window: NatWindow,
    snapshot: Ipv4Peer,
}

impl NatCheck {
    pub(super) fn new(snapshot: Ipv4Peer) -> Self {
        Self {
            pending: 0,
            asked: 0,
            replies: 0,
            window: NatWindow::new(),
            snapshot,
        }
    }

    pub(super) fn track(&mut self) {
        self.asked += 1;
        self.pending += 1;
    }

    /// Only replies that reached the server socket prove the address is reachable from outside.
    pub(super) fn on_reply(&mut self, server_arrival: bool, to: &Ipv4Peer) {
        self.pending = self.pending.saturating_sub(1);
        if !server_arrival {
            return;
        }
        self.replies += 1;
        self.window.add(&to.host, to.port);
    }

    pub(super) fn on_timeout(&mut self) {
        self.pending = self.pending.saturating_sub(1);
    }

    pub(super) fn is_done(&self) -> bool {
        self.pending == 0
    }

    pub(super) fn outcome(&self, nat_host: Option<&str>, server_port: u16) -> Option<Ipv4Peer> {
        if self.asked == 0 {
            return None;
        }
        let need = if self.asked >= CHECK_TARGETS { 3 } else { 1 };
        if self.replies < need {
            return None;
        }
        let host = self.window.host().filter(|h| Some(*h) == nat_host)?;
        let port = self.window.port();
        if port == 0 || port != server_port {
            return None;
        }
        Some(Ipv4Peer {
            host: host.to_owned(),
            port,
        })
    }

    pub(super) fn snapshot(&self) -> &Ipv4Peer {
        &self.snapshot
    }

    pub(super) fn into_window(self) -> NatWindow {
        self.window
    }
}

/// Picks who to ask: the five most recently seen table nodes, topped up with bootstrap nodes
/// (appended without de-duplication, as the reference does).
pub(super) fn check_targets(
    mut nodes: Vec<(u64, Ipv4Peer)>,
    bootstrap: &[(String, u16)],
) -> Vec<Ipv4Peer> {
    nodes.sort_by_key(|(seen, _)| std::cmp::Reverse(*seen));
    let mut out: Vec<Ipv4Peer> = nodes
        .into_iter()
        .take(CHECK_TARGETS)
        .map(|(_, peer)| peer)
        .collect();
    if out.len() >= CHECK_TARGETS {
        return out;
    }
    // The reference adds every bootstrap node once the table gave fewer than five.
    out.extend(bootstrap.iter().map(|(host, port)| Ipv4Peer {
        host: host.clone(),
        port: *port,
    }));
    out
}

/// Parses config bootstrap strings the way `start_bootstrap` does.
pub(super) fn parse_bootstrap(raw: &[String]) -> Vec<(String, u16)> {
    raw.iter().filter_map(|s| parse_bootstrap_str(s)).collect()
}

#[cfg(test)]
#[path = "nat_check_tests.rs"]
mod tests;
