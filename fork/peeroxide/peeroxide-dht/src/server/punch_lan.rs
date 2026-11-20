//! LAN probes of the server's puzzle: when the client sits behind our own public address, the
//! puzzle socket sends it a one-byte probe on every round, so the client's puncher can open the
//! path over the LAN even when the address we announced is unreachable for it.

use std::net::Ipv4Addr;

use super::punch::PunchState;
use crate::hyperdht::is_addr_private;
use crate::messages::Ipv4Peer;

/// At most this many LAN hosts are probed per round.
pub(super) const MAX_LAN_HOSTS: usize = 8;

/// Private hosts of the client's hello worth probing: none unless the client is behind our own
/// address (`same_host`); no loopback, unspecified or repeated hosts; the first
/// [`MAX_LAN_HOSTS`] in hello order.
pub(super) fn lan_hosts(same_host: bool, hello: &[Ipv4Peer]) -> Vec<String> {
    let mut hosts: Vec<String> = Vec::new();
    if !same_host {
        return hosts;
    }
    for a in hello {
        let probeable = a
            .host
            .parse::<Ipv4Addr>()
            .is_ok_and(|ip| !ip.is_loopback() && !ip.is_unspecified());
        if probeable && is_addr_private(&a.host) && !hosts.contains(&a.host) {
            hosts.push(a.host.clone());
        }
        if hosts.len() == MAX_LAN_HOSTS {
            break;
        }
    }
    hosts
}

/// The LAN hosts paired with the external port of the client's puncher socket (the port of
/// `peer_address`; a router that keeps the source port makes it the LAN port as well).
pub(super) fn lan_targets(hosts: &[String], peer_address: &Ipv4Peer) -> Vec<Ipv4Peer> {
    hosts
        .iter()
        .map(|host| Ipv4Peer {
            host: host.clone(),
            port: peer_address.port,
        })
        .collect()
}

impl PunchState {
    /// Sends the one-byte probe from the puzzle socket to every LAN target; a failed send is
    /// only logged. Does nothing once the puzzle runs in its own task.
    pub(super) fn lan_ping(&self, peer_address: &Ipv4Peer, round: u64) {
        let targets = lan_targets(&self.lan_hosts, peer_address);
        if targets.is_empty() {
            return;
        }
        let Some(sr) = self.puncher.as_ref().and_then(|p| p.primary_socket()) else {
            return;
        };
        for to in &targets {
            if let Err(e) = sr.send_holepunch_to(&to.host, to.port, false) {
                tracing::debug!(to = %format!("{}:{}", to.host, to.port), err = %e, "server: lan ping failed");
            }
        }
        let list: Vec<String> = targets
            .iter()
            .map(|t| format!("{}:{}", t.host, t.port))
            .collect();
        tracing::debug!(targets = %list.join(","), round, "server: lan ping sent");
    }
}

#[cfg(test)]
#[path = "punch_lan_ping_tests.rs"]
pub(in crate::server) mod punch_lan_ping_tests;
