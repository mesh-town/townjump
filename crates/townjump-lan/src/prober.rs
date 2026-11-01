//! Prober: asks the LAN who serves a target.

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::Duration;

use socket2::SockRef;
use tokio::net::UdpSocket;
use tokio::time::{Instant, sleep_until};

use crate::LanConfig;
use crate::codec::{self, Nonce, Request, Target};
use crate::interfaces::ipv4_interfaces;

/// When requests are sent and how long the prober waits for a reply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProbeTiming {
    /// Send offsets from the start of the probe.
    pub sends: Vec<Duration>,
    /// Total waiting time; `find` returns `None` when it passes. 3 s by default: the macOS application firewall
    /// delays the first packet of a new flow under CPU load.
    pub window: Duration,
}

impl Default for ProbeTiming {
    fn default() -> Self {
        Self {
            sends: vec![
                Duration::ZERO,
                Duration::from_millis(250),
                Duration::from_millis(750),
            ],
            window: Duration::from_millis(3000),
        }
    }
}

fn open_socket() -> std::io::Result<UdpSocket> {
    let socket = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    socket.set_nonblocking(true)?;
    socket.set_multicast_ttl_v4(1)?;
    socket.set_multicast_loop_v4(true)?;
    UdpSocket::from_std(socket)
}

fn send_all(socket: &UdpSocket, cfg: &LanConfig, packet: &[u8], ifaces: &[Ipv4Addr]) {
    let dest = SocketAddr::V4(SocketAddrV4::new(cfg.group, cfg.port));
    for iface in ifaces {
        if let Err(e) = SockRef::from(socket).set_multicast_if_v4(iface) {
            tracing::debug!("lan: cannot select {iface}: {e}");
            continue;
        }
        if let Err(e) = socket.try_send_to(packet, dest) {
            tracing::debug!("lan: cannot send via {iface}: {e}");
        }
    }
}

fn accept(buf: &[u8], src: SocketAddrV4, nonce: &Nonce, target: &Target) -> Option<SocketAddrV4> {
    let r = codec::decode_reply(buf)?;
    (r.nonce == *nonce && r.target == *target && r.port != 0)
        .then(|| SocketAddrV4::new(*src.ip(), r.port))
}

/// Last look at the deadline: takes a valid reply already sitting in the socket buffer, without waiting.
fn drain(socket: &UdpSocket, nonce: &Nonce, target: &Target) -> Option<SocketAddrV4> {
    let mut buf = [0u8; 256];
    while let Ok((n, SocketAddr::V4(src))) = socket.try_recv_from(&mut buf) {
        if let Some(addr) = accept(&buf[..n], src, nonce, target) {
            return Some(addr);
        }
    }
    None
}

/// Looks for the server of `target` on the LAN. Returns its address (reply
/// source IP, port from the payload) or `None` when nobody answers in time.
/// Never fails: socket errors are logged and end in `None`.
pub async fn find(cfg: &LanConfig, target: Target, timing: &ProbeTiming) -> Option<SocketAddrV4> {
    let socket = open_socket()
        .inspect_err(|e| tracing::debug!("lan: cannot open probe socket: {e}"))
        .ok()?;
    let mut nonce: Nonce = [0; 8];
    getrandom::fill(&mut nonce)
        .inspect_err(|e| tracing::debug!("lan: no randomness: {e}"))
        .ok()?;
    let packet = codec::encode_request(&Request { nonce, target });
    let ifaces = ipv4_interfaces();
    let start = Instant::now();
    let deadline = start + timing.window;
    let mut next = 0;
    let mut buf = [0u8; 256];
    loop {
        let due = timing.sends.get(next).map(|o| start + *o);
        tokio::select! {
            _ = sleep_until(deadline) => return drain(&socket, &nonce, &target),
            _ = sleep_until(due.unwrap_or(deadline)), if due.is_some() => {
                send_all(&socket, cfg, &packet, &ifaces);
                next += 1;
            }
            got = socket.recv_from(&mut buf) => match got {
                Ok((n, SocketAddr::V4(src))) => {
                    if let Some(addr) = accept(&buf[..n], src, &nonce, &target) {
                        return Some(addr);
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::debug!("lan: receive failed: {e}");
                    sleep_until((Instant::now() + Duration::from_millis(10)).min(deadline)).await;
                }
            },
        }
    }
}

#[cfg(test)]
#[path = "prober_tests.rs"]
mod tests;
