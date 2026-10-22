// "Don't fragment" for a UDP socket: requested right after bind and buffers (socket.rs:bind), per platform.
// JS: libudx/src/udx.c:udx_socket_bind (call), libudx/src/io_posix.c, io_win.c:udx__udp_set_dontfrag.
// Related: socket_dontfrag_tests.rs (tests), socket.rs (call from `bind`).

/// A socket option with an `int` value (`int val` in every branch of the reference).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct IntOption {
    level: i32,
    name: i32,
    value: i32,
}

impl IntOption {
    const fn new(level: i32, name: i32, value: i32) -> Self {
        Self { level, name, value }
    }
}

/// The platform's "don't fragment" options, `[v4, v6]`; `None` — platforms the reference does not build on.
/// macOS — `io_posix.c:47-58` (`IP_DONTFRAG`, `IPV6_DONTFRAG` = 1; FreeBSD with the same names). On macOS the kernel accepts the option
/// but does not enforce it: the request is for parity with the reference and for Linux.
#[cfg(any(target_vendor = "apple", target_os = "freebsd"))]
const DONTFRAG: Option<[IntOption; 2]> = Some([
    IntOption::new(libc::IPPROTO_IP, libc::IP_DONTFRAG, 1),
    IntOption::new(libc::IPPROTO_IPV6, libc::IPV6_DONTFRAG, 1),
]);

/// Linux — `io_posix.c:93-104`: `IP_MTU_DISCOVER`/`IPV6_MTU_DISCOVER` = `*_PMTUDISC_PROBE` (DF ignoring ICMP about a smaller MTU).
#[cfg(any(target_os = "linux", target_os = "android"))]
const DONTFRAG: Option<[IntOption; 2]> = Some([
    IntOption::new(
        libc::IPPROTO_IP,
        libc::IP_MTU_DISCOVER,
        libc::IP_PMTUDISC_PROBE,
    ),
    IntOption::new(
        libc::IPPROTO_IPV6,
        libc::IPV6_MTU_DISCOVER,
        libc::IPV6_PMTUDISC_PROBE,
    ),
]);

/// Windows — `io_win.c:60-70`: `IP_MTU_DISCOVER`/`IPV6_MTU_DISCOVER` = `IP_PMTUDISC_PROBE` for both families.
#[cfg(windows)]
const DONTFRAG: Option<[IntOption; 2]> = Some([
    IntOption::new(ws::IPPROTO_IP, ws::IP_MTU_DISCOVER, ws::IP_PMTUDISC_PROBE),
    IntOption::new(
        ws::IPPROTO_IPV6,
        ws::IPV6_MTU_DISCOVER,
        ws::IP_PMTUDISC_PROBE,
    ),
]);

#[cfg(not(any(
    target_vendor = "apple",
    target_os = "freebsd",
    target_os = "linux",
    target_os = "android",
    windows
)))]
const DONTFRAG: Option<[IntOption; 2]> = None;

#[cfg(windows)]
use windows_sys::Win32::Networking::WinSock as ws;

/// The option by socket family (`socket->family == 6` of the reference), not by destination.
fn dontfrag_option(is_ipv6: bool) -> Option<IntOption> {
    DONTFRAG.map(|options| options[usize::from(is_ipv6)])
}

/// The "don't fragment" request right after the buffers, as in the reference (`udx.c:2095-2098`): a kernel refusal is only a debug-level record.
pub(super) fn configure_dontfrag(udp: &tokio::net::UdpSocket, is_ipv6: bool) {
    request_dontfrag(|| set_dontfrag(udp, is_ipv6));
}

/// Performs the setup; a refusal is logged at debug and returns `false`, bind goes on.
fn request_dontfrag(set: impl FnOnce() -> std::io::Result<()>) -> bool {
    let Err(refusal) = set() else { return true };
    tracing::debug!(%refusal, "failed to set IP Don't Fragment socket option");
    false
}

fn set_dontfrag(udp: &tokio::net::UdpSocket, is_ipv6: bool) -> std::io::Result<()> {
    let option = dontfrag_option(is_ipv6).ok_or(std::io::ErrorKind::Unsupported)?;
    set_int_option(udp, option)
}

#[cfg(unix)]
#[allow(unsafe_code)] // the only unsafe in the crate: socket2 0.6 does not expose this option (see lib.rs); in the test — reading the same option
fn set_int_option(udp: &tokio::net::UdpSocket, option: IntOption) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;
    let value: libc::c_int = option.value;
    let len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
    // SAFETY: (1) the descriptor comes from `as_raw_fd()` on a reference to a bound `UdpSocket` — no ownership, no close, it is alive while
    // `udp` is alive, for the duration of the call; (2) `value` is a `c_int` on the stack, the pointer to it and `len = size_of::<c_int>()` are exactly what
    // the kernel expects for IP_DONTFRAG/IP_MTU_DISCOVER (type `int`); (3) the result is checked below: 0 — success, otherwise `last_os_error`.
    let rc = unsafe {
        libc::setsockopt(
            udp.as_raw_fd(),
            option.level,
            option.name,
            (&raw const value).cast(),
            len,
        )
    };
    match rc {
        0 => Ok(()),
        _ => Err(std::io::Error::last_os_error()),
    }
}

#[cfg(windows)]
#[allow(unsafe_code)] // see the unix variant above
fn set_int_option(udp: &tokio::net::UdpSocket, option: IntOption) -> std::io::Result<()> {
    use std::os::windows::io::AsRawSocket;
    let value: i32 = option.value;
    let len = std::mem::size_of::<i32>() as i32;
    // SAFETY: (1) the socket comes from `as_raw_socket()` on a reference to a bound `UdpSocket` — no ownership, no close, it is alive while
    // `udp` is alive, for the duration of the call; (2) `value` is an `int` on the stack, the pointer to it and `len = size_of::<i32>()` are what Winsock expects
    // for IP_MTU_DISCOVER/IPV6_MTU_DISCOVER (type `int`); (3) the result is checked below: 0 — success, otherwise `last_os_error`.
    let rc = unsafe {
        ws::setsockopt(
            udp.as_raw_socket() as ws::SOCKET,
            option.level,
            option.name,
            (&raw const value).cast(),
            len,
        )
    };
    match rc {
        0 => Ok(()),
        _ => Err(std::io::Error::last_os_error()),
    }
}

#[cfg(not(any(unix, windows)))]
fn set_int_option(_udp: &tokio::net::UdpSocket, _option: IntOption) -> std::io::Result<()> {
    Err(std::io::ErrorKind::Unsupported.into())
}

#[cfg(test)]
#[path = "socket_dontfrag_tests.rs"]
mod tests;
