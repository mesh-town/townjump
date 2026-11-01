//! Enumeration of the local IPv4 interfaces used for multicast.

use std::net::Ipv4Addr;

/// Addresses of all non-loopback IPv4 interfaces, without duplicates.
/// An enumeration failure is logged and yields an empty list.
pub fn ipv4_interfaces() -> Vec<Ipv4Addr> {
    let all = match if_addrs::get_if_addrs() {
        Ok(all) => all,
        Err(e) => {
            tracing::debug!("lan: cannot list interfaces: {e}");
            return Vec::new();
        }
    };
    let mut out: Vec<Ipv4Addr> = Vec::new();
    for iface in all {
        if let std::net::IpAddr::V4(ip) = iface.ip()
            && !iface.is_loopback()
            && !out.contains(&ip)
        {
            out.push(ip);
        }
    }
    out
}
