//! Picks the advertised remote address on one of our own subnets (LAN dial target).
//! Used by `finish_connection` when both peers share a public host.

use std::net::Ipv4Addr;

use crate::messages::Ipv4Peer;

use super::connect_finish::is_addr_private;

/// All IPv4 interfaces as `(ip, prefixlen)`; empty on enumeration error.
/// Loopback is kept: a remote advertising `127.x` (loopback clusters) stays the first choice.
pub(super) fn local_v4_nets() -> Vec<(Ipv4Addr, u8)> {
    let Ok(ifs) = if_addrs::get_if_addrs() else {
        return Vec::new();
    };
    ifs.into_iter()
        .filter_map(|i| match i.addr {
            if_addrs::IfAddr::V4(v4) => Some((v4.ip, v4.prefixlen)),
            _ => None,
        })
        .collect()
}

fn same_net(a: Ipv4Addr, ip: Ipv4Addr, prefixlen: u8) -> bool {
    if prefixlen == 0 || prefixlen > 32 {
        return false;
    }
    let mask = u32::MAX << (32 - u32::from(prefixlen));
    u32::from(a) & mask == u32::from(ip) & mask
}

/// Our own non-loopback interface address (loopback stays valid for loopback clusters).
fn is_own_address(a: Ipv4Addr, nets: &[(Ipv4Addr, u8)]) -> bool {
    !a.is_loopback() && nets.iter().any(|&(ip, _)| ip == a)
}

fn is_lan_candidate(p: &Ipv4Peer, local_nets: &[(Ipv4Addr, u8)]) -> bool {
    let Ok(a) = p.host.parse::<Ipv4Addr>() else {
        return false;
    };
    is_addr_private(&p.host)
        && !is_own_address(a, local_nets)
        && local_nets.iter().any(|&(ip, len)| same_net(a, ip, len))
}

/// First advertised private address sharing a subnet with one of `local_nets`.
/// Our own interface addresses are skipped: default bridge addresses (docker0, virtual-machine bridges,
/// libvirt) repeat on every host, so such an advertised address is our bridge, not the peer.
pub(super) fn pick_lan_addr(
    advertised: &[Ipv4Peer],
    local_nets: &[(Ipv4Addr, u8)],
) -> Option<Ipv4Peer> {
    advertised
        .iter()
        .find(|p| is_lan_candidate(p, local_nets))
        .cloned()
}

#[cfg(test)]
#[path = "lan_pick_tests.rs"]
mod tests;
