//! Enumeration of the local IPv4 interfaces used for multicast.

use std::fmt;
use std::net::Ipv4Addr;
use std::sync::Arc;

/// Where the responder learns the local interfaces from: the system, or a substitute in tests.
#[derive(Clone)]
pub struct InterfaceSource(Arc<dyn Fn() -> Vec<Ipv4Addr> + Send + Sync>);

impl InterfaceSource {
    /// The interfaces of this host, as `ipv4_interfaces` lists them.
    pub fn system() -> Self {
        Self::new(ipv4_interfaces)
    }

    /// A source backed by `f`.
    pub fn new(f: impl Fn() -> Vec<Ipv4Addr> + Send + Sync + 'static) -> Self {
        Self(Arc::new(f))
    }

    /// The current list of interface addresses.
    pub fn list(&self) -> Vec<Ipv4Addr> {
        (self.0)()
    }
}

impl fmt::Debug for InterfaceSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("InterfaceSource")
    }
}

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
