// JS: hyperdht/lib/holepuncher.js (localAddresses), hyperdht/lib/nat.js (addresses)
//! Address lists of a [`Holepuncher`]: local interface addresses and the
//! holepunch payload `addresses` field.

use super::Holepuncher;
use crate::messages::Ipv4Peer;

impl Holepuncher {
    /// Enumerate IPv4 addresses of local network interfaces, paired with
    /// the supplied port, for advertising to remote peers. Mirrors Node
    /// `hyperdht/lib/holepuncher.js Holepuncher.localAddresses` — keeps
    /// only `family === 4 && !internal` (i.e. skips loopback). Falls back
    /// to `[127.0.0.1:port]` when no usable interface is found.
    pub fn local_addresses(port: u16) -> Vec<Ipv4Peer> {
        let mut out = Vec::new();
        if let Ok(ifs) = if_addrs::get_if_addrs() {
            for iface in ifs {
                if iface.is_loopback() {
                    continue;
                }
                if let std::net::IpAddr::V4(v4) = iface.ip() {
                    out.push(Ipv4Peer {
                        host: v4.to_string(),
                        port,
                    });
                }
            }
        }
        if out.is_empty() {
            out.push(Ipv4Peer {
                host: "127.0.0.1".to_string(),
                port,
            });
        }
        out
    }

    /// Address list for the holepunch payload's `addresses` field: a copy of
    /// the reflexive samples in `self.nat.addresses`, empty when there are
    /// none (JS `addresses: c.puncher.nat.addresses`, `hyperdht/lib/connect.js`
    /// and `server.js`). Interface addresses are deliberately not included:
    /// the peer would probe them too and the first reply may come through a
    /// VPN tunnel. A same-LAN pair is handled by `same_host`
    /// and the handshake `addresses4` (see [`Self::handshake_addresses4`]).
    pub fn punch_addresses(&self) -> Vec<Ipv4Peer> {
        Self::select_punch_addresses(self.nat.addresses.as_ref())
    }

    pub(crate) fn select_punch_addresses(nat_addresses: Option<&Vec<Ipv4Peer>>) -> Vec<Ipv4Peer> {
        nat_addresses.cloned().unwrap_or_default()
    }

    /// Handshake `addresses4`: the payload samples followed by the local
    /// interface addresses of the puzzle socket (`port`), for same-LAN
    /// direct connects.
    pub(crate) fn handshake_addresses4(punch_addrs: &[Ipv4Peer], port: u16) -> Vec<Ipv4Peer> {
        let mut out = punch_addrs.to_vec();
        out.extend(Self::local_addresses(port));
        out
    }
}

#[cfg(test)]
#[path = "addresses_tests.rs"]
mod tests;

pub fn match_address(
    local_addresses: &[Ipv4Peer],
    remote_local_addresses: &[Ipv4Peer],
) -> Option<Ipv4Peer> {
    if remote_local_addresses.is_empty() {
        return None;
    }

    let mut best_segment = 1u8;
    let mut best_addr: Option<&Ipv4Peer> = None;

    for local in local_addresses {
        let a: Vec<&str> = local.host.split('.').collect();
        if a.len() != 4 {
            continue;
        }

        for remote in remote_local_addresses {
            let b: Vec<&str> = remote.host.split('.').collect();
            if b.len() != 4 {
                continue;
            }

            if a[0] == b[0] {
                if best_segment == 1 {
                    best_segment = 2;
                    best_addr = Some(remote);
                }

                if a[1] == b[1] {
                    if best_segment == 2 {
                        best_segment = 3;
                        best_addr = Some(remote);
                    }

                    if a[2] == b[2] {
                        return Some(remote.clone());
                    }
                }
            }
        }
    }

    best_addr.cloned()
}
