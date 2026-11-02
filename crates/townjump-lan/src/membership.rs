//! Membership of the responder socket in the multicast group, per interface.

use std::collections::HashSet;
use std::io::ErrorKind;
use std::net::Ipv4Addr;

use tokio::net::UdpSocket;

/// Brings `joined` in line with `current`: forgets interfaces that vanished (the kernel
/// dropped their membership) and joins the new ones. A failed join is retried on the next call.
pub(crate) fn sync(
    socket: &UdpSocket,
    group: Ipv4Addr,
    joined: &mut HashSet<Ipv4Addr>,
    current: &[Ipv4Addr],
) {
    joined.retain(|iface| current.contains(iface));
    let fresh: Vec<Ipv4Addr> = current
        .iter()
        .copied()
        .filter(|i| !joined.contains(i))
        .collect();
    joined.extend(
        fresh
            .into_iter()
            .filter(|iface| join(socket, group, *iface)),
    );
}

fn join(socket: &UdpSocket, group: Ipv4Addr, iface: Ipv4Addr) -> bool {
    match socket.join_multicast_v4(group, iface) {
        Ok(()) => true,
        Err(e) if e.kind() == ErrorKind::AddrInUse => true,
        Err(e) => {
            tracing::debug!("lan: cannot join {group} on {iface}: {e}");
            false
        }
    }
}
