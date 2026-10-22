// JS: libudx/src/udx.c:on_uv_udp_recv; libuv src/win/udp.c:uv__process_udp_recv_req (WSAECONNRESET/WSAENETRESET — skipped)
// Receiving a datagram while skipping the "send echo": on Windows an ICMP "port unreachable" for our send surfaces as an error
// of the next `recv_from` (10054). This is not the end of the socket: the loop keeps reading. On POSIX an unconnected socket does not see ICMP, and
// `ECONNREFUSED` of a connected one is a rejection, it is fatal (see `other_kinds_are_fatal`).
use std::net::SocketAddr;
use std::time::Duration;

use tokio::net::UdpSocket;
use tracing::{debug, trace, warn};

#[cfg(windows)]
use windows_sys::Win32::Networking::WinSock as ws;

/// Up to this many consecutive skips are made without pausing, so a batch of them passes without delay.
const SKIPS_BEFORE_PAUSE: u32 = 256;
/// A pause on each skip above the threshold: a permanent error gives ~100 wakeups/s instead of a busy core.
const SKIP_PAUSE: Duration = Duration::from_millis(10);

/// A receive error here is an echo of our own send to a dead port, not a socket failure.
/// 10053 (`ConnectionAborted`) is skipped beyond libuv (`udp.c` stops reading on it): the node would go deaf anyway,
/// and skipping costs only the `SKIP_PAUSE` pause on a permanent error.
fn is_send_echo(e: &std::io::Error) -> bool {
    use std::io::ErrorKind;
    let by_kind = matches!(
        e.kind(),
        ErrorKind::ConnectionReset | ErrorKind::ConnectionAborted
    );
    #[cfg(windows)]
    let by_code = e.raw_os_error() == Some(ws::WSAENETRESET);
    #[cfg(not(windows))]
    let by_code = false;
    by_kind || by_code
}

/// Pause before the next attempt after `skipped` skips in a row.
fn pause_after(skipped: u32) -> Option<Duration> {
    (skipped >= SKIPS_BEFORE_PAUSE).then_some(SKIP_PAUSE)
}

/// `recv_from` until success; skippable errors are counted and reading goes on, a fatal one — `warn!` and `None`.
pub(super) async fn recv_next(udp: &UdpSocket, buf: &mut [u8]) -> Option<(usize, SocketAddr)> {
    let mut skipped: u32 = 0;
    loop {
        match udp.recv_from(buf).await {
            Ok(got) => return Some(got),
            Err(e) if is_send_echo(&e) => {
                skipped = skipped.saturating_add(1);
                trace!(target: "udx::rx", err = %e, skipped, "receive: send echo skipped");
                if skipped == SKIPS_BEFORE_PAUSE {
                    debug!(target: "udx::rx", skipped, "receive: many echoes in a row, pausing from now on");
                }
                if let Some(pause) = pause_after(skipped) {
                    tokio::time::sleep(pause).await;
                }
            }
            Err(e) => {
                warn!(target: "udx::rx", err = %e, "receive stopped: fatal recv_from error");
                return None;
            }
        }
    }
}

#[cfg(test)]
#[path = "socket_recv_tests.rs"]
mod tests;
