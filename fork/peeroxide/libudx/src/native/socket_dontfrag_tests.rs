// Tests of "don't fragment" (socket_dontfrag.rs): after `bind` the socket requests the option like the reference, a kernel refusal does not break bind.
//
// What they guard: "we request like the reference", not "there is no fragmentation". `getsockopt` covers only the first fact — the kernel accepted the option.
// On macOS the kernel accepts the option but does NOT enforce it (datagrams are fragmented with or without it,
// including on a connected socket). On Linux it does take effect:
// a datagram above the interface MTU gets EMSGSIZE, without the option it goes out fragmented.
use std::io;

use super::*;

#[test]
fn refused_option_is_logged_not_raised() {
    let mut calls = 0;
    let accepted = request_dontfrag(|| {
        calls += 1;
        Err(io::Error::from_raw_os_error(22))
    });
    assert_eq!((accepted, calls), (false, 1));
}

#[test]
fn accepted_option_is_reported_once() {
    let mut calls = 0;
    let accepted = request_dontfrag(|| {
        calls += 1;
        Ok(())
    });
    assert_eq!((accepted, calls), (true, 1));
}

#[cfg(any(
    target_vendor = "apple",
    target_os = "freebsd",
    target_os = "linux",
    target_os = "android"
))]
mod kernel {
    use std::sync::Arc;

    use tokio::net::UdpSocket;

    use super::*;
    use crate::native::socket::UdxSocket;

    /// Expectation `[v4, v6]` from system headers (numbers, not `libc` names): (level, name, value).
    /// macOS: netinet/in.h:436 `IP_DONTFRAG` 28, netinet6/in6.h:494 `IPV6_DONTFRAG` 62, value 1.
    #[cfg(target_vendor = "apple")]
    const EXPECTED: [(i32, i32, i32); 2] = [(libc::IPPROTO_IP, 28, 1), (libc::IPPROTO_IPV6, 62, 1)];
    /// FreeBSD: netinet/in.h `IP_DONTFRAG` 67, netinet6/in6.h `IPV6_DONTFRAG` 62.
    #[cfg(target_os = "freebsd")]
    const EXPECTED: [(i32, i32, i32); 2] = [(libc::IPPROTO_IP, 67, 1), (libc::IPPROTO_IPV6, 62, 1)];
    /// Linux: linux/in.h `IP_MTU_DISCOVER` 10, linux/in6.h `IPV6_MTU_DISCOVER` 23, `*_PMTUDISC_PROBE` 3.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    const EXPECTED: [(i32, i32, i32); 2] = [(libc::IPPROTO_IP, 10, 3), (libc::IPPROTO_IPV6, 23, 3)];

    const V4: usize = 0;
    const V6: usize = 1;

    #[allow(unsafe_code)] // reading the option from the test; in the crate itself unsafe is allowed only in socket_dontfrag.rs
    fn read_option(udp: &UdpSocket, level: i32, name: i32) -> io::Result<i32> {
        use std::os::fd::AsRawFd;
        let mut value: libc::c_int = -1;
        let mut len = size_of::<libc::c_int>() as libc::socklen_t;
        // SAFETY: the descriptor comes from `as_raw_fd()` on a reference — no ownership, no close, alive while `udp` is alive; `value` is a `c_int`
        // on the stack, `len` is its size, exactly what the kernel returns for `int` options; the result is checked below.
        let rc = unsafe {
            libc::getsockopt(
                udp.as_raw_fd(),
                level,
                name,
                (&raw mut value).cast(),
                &mut len,
            )
        };
        match rc {
            0 => Ok(value),
            _ => Err(io::Error::last_os_error()),
        }
    }

    fn requested(udp: &UdpSocket, family: usize) -> i32 {
        let (level, name, _) = EXPECTED[family];
        read_option(udp, level, name).unwrap()
    }

    async fn bound(addr: &str) -> Arc<UdpSocket> {
        let socket = UdxSocket::new();
        socket.bind(addr.parse().unwrap()).await.unwrap();
        socket.udp_arc().unwrap()
    }

    #[tokio::test]
    async fn v4_socket_asks_dontfrag_after_bind() {
        let udp = bound("127.0.0.1:0").await;
        assert_eq!(requested(&udp, V4), EXPECTED[V4].2);
    }

    #[tokio::test]
    async fn v6_socket_asks_dontfrag_after_bind() {
        let udp = bound("[::1]:0").await;
        assert_eq!(requested(&udp, V6), EXPECTED[V6].2);
    }

    /// Read control: on a socket that bypassed `bind` the option is not set yet — otherwise the previous tests would prove nothing.
    #[tokio::test]
    async fn fresh_socket_does_not_have_the_option() {
        let udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        assert_ne!(requested(&udp, V4), EXPECTED[V4].2);
    }

    #[test]
    fn option_table_is_the_reference_table() {
        let want = |(level, name, value)| Some(IntOption::new(level, name, value));
        assert_eq!(dontfrag_option(false), want(EXPECTED[V4]));
        assert_eq!(dontfrag_option(true), want(EXPECTED[V6]));
    }

    /// The branch follows the socket family: the kernel rejects a v6 option on a v4 socket, and bind does not suffer from it.
    #[tokio::test]
    async fn wrong_family_option_is_refused_by_kernel_and_survived() {
        let udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        assert!(set_dontfrag(&udp, true).is_err());
        configure_dontfrag(&udp, true);
        assert!(set_dontfrag(&udp, false).is_ok());
    }

    /// "Takes effect", not "accepted": a datagram above the MTU of the outgoing interface on our socket is rejected by the kernel (EMSGSIZE), while on a regular
    /// socket without the option it goes out fragmented. A route not via lo is needed, hence the `ignore`d test: run on a host with such a route (for example under netem) —
    /// `DONTFRAG_PROBE_DST=<address behind eth0> libudx-<hash> --ignored oversized`.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    #[ignore = "needs a route not via lo: DONTFRAG_PROBE_DST=<address behind eth0>, run with --ignored"]
    async fn oversized_datagram_is_refused_instead_of_fragmented() {
        let dst = std::env::var("DONTFRAG_PROBE_DST").expect("DONTFRAG_PROBE_DST");
        let dst: std::net::SocketAddr = format!("{dst}:9").parse().unwrap();
        let big = vec![0u8; 16_000];
        let refused = bound("0.0.0.0:0")
            .await
            .send_to(&big, dst)
            .await
            .unwrap_err();
        assert_eq!(refused.raw_os_error(), Some(libc::EMSGSIZE));
        let control = UdpSocket::bind("0.0.0.0:0").await.unwrap();
        assert_eq!(control.send_to(&big, dst).await.unwrap(), big.len());
    }
}
