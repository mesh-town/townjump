// JS: libudx/src/io_posix.c:42-46, 62-89
//! Link MTU to an address as the kernel sees it: the reference's `udx__get_link_mtu`. On Linux/Android — a temporary UDP socket,
//! `connect`, `getsockopt(IP_MTU / IPV6_MTU)`; on other systems — `None`, like the reference on macOS and FreeBSD.
//! Windows is not ported.

use std::net::SocketAddr;

#[cfg(any(target_os = "linux", target_os = "android"))]
pub(crate) fn link_mtu(addr: SocketAddr) -> Option<usize> {
    use socket2::{Domain, Protocol, Socket, Type};

    let domain = if addr.is_ipv6() {
        Domain::IPV6
    } else {
        Domain::IPV4
    };
    let socket = Socket::new(domain, Type::DGRAM, Some(Protocol::UDP)).ok()?;
    // The socket closes on leaving the function and on a `connect` failure too: the reference leaks the descriptor there
    // (io_posix.c:70-73, `return -1` without `close`) — an intentional divergence, it does not affect stream behavior.
    socket.connect(&addr.into()).ok()?;
    read_ip_mtu(&socket, addr.is_ipv6())
}

#[cfg(any(target_os = "linux", target_os = "android"))]
#[allow(unsafe_code)] // see lib.rs: targeted unsafe, as in socket_dontfrag.rs
fn read_ip_mtu(socket: &socket2::Socket, is_ipv6: bool) -> Option<usize> {
    use std::os::fd::AsRawFd;
    let (level, name) = if is_ipv6 {
        (libc::IPPROTO_IPV6, libc::IPV6_MTU)
    } else {
        (libc::IPPROTO_IP, libc::IP_MTU)
    };
    let mut value: libc::c_int = 0;
    let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
    // SAFETY: (1) the descriptor comes from `as_raw_fd()` on a reference to a live `Socket`, without ownership or closing; (2) `value` is a `c_int`
    // on the stack, `len = size_of::<c_int>()` is what the kernel writes for IP_MTU/IPV6_MTU (type `int`); (3) the result is checked:
    // 0 — success, otherwise `None`.
    let rc = unsafe {
        libc::getsockopt(
            socket.as_raw_fd(),
            level,
            name,
            (&raw mut value).cast(),
            &raw mut len,
        )
    };
    if rc == 0 && value > 0 {
        Some(value as usize)
    } else {
        None
    }
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
pub(crate) fn link_mtu(_addr: SocketAddr) -> Option<usize> {
    None
}
