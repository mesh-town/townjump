// JS: bare-dgram → udx-native → libuv src/win/udp.c:uv__process_udp_recv_req (WSAECONNRESET/WSAENETRESET skipped, WSAEMSGSIZE delivered as a cut datagram)
// Related: udp_proxy.rs and udp_server.rs (the two receive loops that read through `recv_next`).
use std::io;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::time::sleep;
use townjump_logger::Logger;

/// Echoes in a row (within one `recv_next` call) after which the loop starts to pause between reads.
const SKIPS_BEFORE_PAUSE: u32 = 256;
/// Pause between reads while echoes keep coming; keeps a flood of ICMP errors from spinning a core.
const SKIP_PAUSE: Duration = Duration::from_millis(10);

/// The error is the echo of our own send, not a failure of the receive: Windows reports an ICMP
/// "port unreachable" for an earlier `send_to` as an error of the next `recv_from`
/// (WSAECONNRESET 10054, WSAECONNABORTED 10053, WSAENETRESET 10052).
fn is_send_echo(e: &io::Error) -> bool {
    if matches!(
        e.kind(),
        io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionAborted
    ) {
        return true;
    }
    // WSAENETRESET (10052): std has no `ErrorKind` for it.
    cfg!(windows) && e.raw_os_error() == Some(10052)
}

/// Like `bare-dgram` over `udx-native`: a received datagram longer than 2048 bytes is truncated.
pub(crate) const DATAGRAM_LIMIT: usize = 2048;
/// Size of the buffer `recv_next` reads into: larger than any UDP datagram (65507 bytes), so the
/// OS never truncates. On Windows `recv_from` into a smaller buffer fails with WSAEMSGSIZE (10040)
/// and loses the data and the sender address, while libuv delivers the cut datagram; the cut to
/// `DATAGRAM_LIMIT` is therefore made after reception.
pub(crate) const RECV_BUF: usize = 64 * 1024;

fn pause_after(skipped: u32) -> Option<Duration> {
    (skipped >= SKIPS_BEFORE_PAUSE).then_some(SKIP_PAUSE)
}

/// `recv_from` that skips the echoes of our own sends and returns on a datagram or a real error.
/// libuv skips 10054 and 10052; on 10053 it stops receiving and libudx drops the error, so the JS
/// socket goes silently deaf. Here 10053 is skipped too (a deliberate deviation: a deaf socket is
/// worse). Cancel-safe: a dropped call loses no datagram. `buf` must be `RECV_BUF` long; on success
/// the datagram is in `buf[..n]`, `n` is cut to `DATAGRAM_LIMIT` (the rest is dropped).
pub(crate) async fn recv_next(
    socket: &UdpSocket,
    buf: &mut [u8],
    logger: &Logger,
) -> io::Result<(usize, SocketAddr)> {
    debug_assert!(
        buf.len() >= RECV_BUF,
        "receive buffer shorter than RECV_BUF"
    );
    let mut skipped: u32 = 0;
    loop {
        let e = match socket.recv_from(buf).await {
            Ok((n, from)) => return Ok((n.min(DATAGRAM_LIMIT), from)),
            Err(e) if is_send_echo(&e) => e,
            Err(e) => return Err(e),
        };
        skipped = skipped.saturating_add(1);
        logger.debug(&format!("UDP receive: skipped send echo: {e}"));
        if skipped == SKIPS_BEFORE_PAUSE {
            logger.debug("UDP receive: many echoes in a row, pausing");
        }
        if let Some(pause) = pause_after(skipped) {
            sleep(pause).await;
        }
    }
}

#[cfg(test)]
#[path = "udp_recv_tests.rs"]
mod tests;
