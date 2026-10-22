// Datagram send with a count and a visible error. The reference (libudx/src/udx.c:727-729) prints any error other than EAGAIN;
// every send branch here reports through the same counters and log. The caller still discards the result explicitly.
//
// The count is taken at the success point: "formed" is counted by `register_sent`, and the difference from `TX_OK` is what was dropped, independent of the log.
// The count is double: datagrams and bytes. Datagrams answer "how often sending fails", bytes answer "how much data
// did not go out as a result". All six send branches send one datagram per call (no aggregation),
// so the two shares differ only by the size distribution: a failure on a data packet (~MTU) weighs as much as a dozen service ones.
// The log is rate-limited: on RTO the window goes out as an unpaced batch and errors arrive in bursts, so a line for each would slow the sender down.
use std::net::SocketAddr;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use tokio::net::UdpSocket;

static TX_OK: AtomicU64 = AtomicU64::new(0);
static TX_ERR: AtomicU64 = AtomicU64::new(0);
static TX_OK_BYTES: AtomicU64 = AtomicU64::new(0);
static TX_ERR_BYTES: AtomicU64 = AtomicU64::new(0);
/// Milliseconds (since the first call, at least 1) of the last written line; 0 — nothing written yet.
static LAST_REPORT_MS: AtomicU64 = AtomicU64::new(0);
/// No more than one log line per this period.
const REPORT_EVERY_MS: u64 = 5_000;

/// Process-wide counters `(successful, dropped)` of datagram sends.
pub fn tx_counters() -> (u64, u64) {
    (
        TX_OK.load(Ordering::Relaxed),
        TX_ERR.load(Ordering::Relaxed),
    )
}

/// The same sends in bytes: `(went to the kernel, dropped)`. Dropped is the length of the buffer that was not accepted.
pub fn tx_bytes() -> (u64, u64) {
    (
        TX_OK_BYTES.load(Ordering::Relaxed),
        TX_ERR_BYTES.load(Ordering::Relaxed),
    )
}

/// Time to write a line: the first error (`last_ms == 0`) at once, then no sooner than 5 s later.
fn should_report(now_ms: u64, last_ms: u64) -> bool {
    last_ms == 0 || now_ms.saturating_sub(last_ms) >= REPORT_EVERY_MS
}

fn now_ms() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    (START.get_or_init(Instant::now).elapsed().as_millis() as u64).max(1)
}

/// The one debug trace of background sends: where a UDX packet leaves from and to (id and flags from its header).
fn trace_send(udp: &UdpSocket, buf: &[u8], addr: SocketAddr, what: &'static str) {
    if !tracing::enabled!(tracing::Level::DEBUG) {
        return;
    }
    let header = (buf.len() >= 20 && buf[0] == 0xff).then(|| {
        let id = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
        (id, buf[2])
    });
    let (dest_id, flags) = header.unwrap_or((0, 0));
    let from = udp.local_addr().ok();
    tracing::debug!(%addr, ?from, dest_id, flags, len = buf.len(), what, "udx send");
}

/// `send_to` with a count; a failure is logged (`udx::tx`, warn) first and then no more than once per 5 s. `true` — it went to the kernel.
pub(crate) async fn send_counted(
    udp: &UdpSocket,
    buf: &[u8],
    addr: SocketAddr,
    what: &'static str,
) -> bool {
    trace_send(udp, buf, addr, what);
    match udp.send_to(buf, addr).await {
        Ok(n) => {
            TX_OK.fetch_add(1, Ordering::Relaxed);
            TX_OK_BYTES.fetch_add(n as u64, Ordering::Relaxed);
            true
        }
        Err(e) => {
            TX_ERR.fetch_add(1, Ordering::Relaxed);
            TX_ERR_BYTES.fetch_add(buf.len() as u64, Ordering::Relaxed);
            let now = now_ms();
            let last = LAST_REPORT_MS.load(Ordering::Relaxed);
            if should_report(now, last)
                && LAST_REPORT_MS
                    .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
                    .is_ok()
            {
                let (ok, dropped) = tx_counters();
                let (ok_b, dropped_b) = tx_bytes();
                tracing::warn!(target: "udx::tx", what, %addr, err = %e, ok, dropped, ok_b, dropped_b, "datagrams not sent");
            }
            false
        }
    }
}

#[cfg(test)]
#[path = "tx_err_tests.rs"]
mod tests;
