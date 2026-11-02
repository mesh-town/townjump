// JS: libudx/src/udx.c (udx.c:442 leaves TIME_WAIT as a TODO; the fork keeps a closed stream's entry as a tombstone)
//! Closed-stream tombstone (TIME_WAIT): after the processor ends, the stream's entry in the socket table stays for
//! `clamp(4·rto, 5 s, 15 s)` and answers late DATA/END of the recorded peer; see `processor_task.rs` (`Grave`).
//!
//! Limit: `change_remote` to a socket whose table holds a tombstone with the same id fails with "stream id taken
//! on target socket" until the tombstone expires; ids come from `next_stream_id`, a repeat is possible only for manual ids.
//! A packet from another address to a tombstone id is not forwarded to the fallback channel (it is silence).

use std::sync::Weak;
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::runtime::Handle;

use super::*;
use crate::native::stream::seq::Seq;
use crate::native::stream::wire::ack_packet;

pub(super) const LINGER_MIN: Duration = Duration::from_secs(20);
pub(super) const LINGER_MAX: Duration = Duration::from_secs(60);

/// Tombstone lifetime: the peer repeats every `2·rto` up to `13·rto`, so `14·rto` covers the whole series;
/// `clamp(14·rto, 20 s, 60 s)`, `rto` as it was at the close.
pub(super) fn tomb_ttl(rto: Duration) -> Duration {
    rto.saturating_mul(14).max(LINGER_MIN).min(LINGER_MAX)
}

/// What the tombstone remembers about the closed stream.
pub(super) struct Tomb {
    pub(super) remote_addr: SocketAddr,
    pub(super) remote_id: u32,
    pub(super) udp: Weak<UdpSocket>,
    pub(super) final_seq: u32,
    pub(super) final_ack: u32,
    pub(super) end_acked: bool,
    pub(super) remote_ended: bool,
    pub(super) rto: Duration,
    /// Replies sent so far (the `sent` field of the `tombstone reply` log line).
    pub(super) sent: u32,
    pub(super) until: tokio::time::Instant,
}

#[derive(Debug, PartialEq)]
pub(super) enum Reply {
    Ack(Vec<u8>),
    Destroy(Vec<u8>),
    Silent,
}

fn refuse(local_id: u32, reason: &'static str) -> Option<Tomb> {
    tracing::debug!(local_id, reason, "tombstone refused");
    None
}

impl Tomb {
    /// `None` when no tombstone is due (the reason goes to the debug log); `same_channel` is checked by `bury`.
    pub(super) fn from_inner(
        inner: &StreamInner,
        home: &Weak<UdpSocket>,
        local_id: u32,
    ) -> Option<Tomb> {
        let Some(remote_addr) = inner.remote_addr else {
            return refuse(local_id, "no_remote_addr");
        };
        let (Some(udp), Some(home_udp)) = (inner.udp.as_ref(), home.upgrade()) else {
            return refuse(local_id, "no_udp");
        };
        if inner.relay_target.is_some() {
            return refuse(local_id, "relay");
        }
        if !Arc::ptr_eq(udp, &home_udp) {
            return refuse(local_id, "socket_changed");
        }
        let rto = Duration::from_millis(u64::from(inner.rto));
        Some(Tomb {
            remote_addr,
            remote_id: inner.remote_id,
            udp: home.clone(),
            final_seq: inner.next_seq,
            final_ack: inner.next_remote_seq,
            end_acked: inner.end_acked,
            remote_ended: inner.remote_ended,
            rto,
            sent: 0,
            until: tokio::time::Instant::now() + tomb_ttl(rto),
        })
    }

    /// The answer to a packet: only DATA/END from the recorded address; DESTROY, ACK, HEARTBEAT, MESSAGE get silence.
    pub(super) fn reply(&self, hdr: &Header, from: SocketAddr) -> Reply {
        let flags = hdr.type_flags;
        if from != self.remote_addr
            || flags & FLAG_DESTROY != 0
            || flags & (FLAG_DATA | FLAG_END) == 0
        {
            return Reply::Silent;
        }
        let in_order_end = flags & FLAG_END != 0 && hdr.seq == self.final_ack;
        if self.end_acked && (self.remote_ended || in_order_end) {
            let ack = Seq(self.final_ack).max(Seq(hdr.seq.wrapping_add(1))).0;
            return Reply::Ack(ack_packet(self.remote_id, self.final_seq, ack, &[]));
        }
        let header = Header {
            type_flags: FLAG_DESTROY,
            data_offset: 0,
            remote_id: self.remote_id,
            recv_window: 0,
            seq: self.final_seq,
            ack: self.final_ack,
        };
        Reply::Destroy(header.encode().to_vec())
    }
}

/// Puts the tombstone in the entry's place and starts its task; at any refusal the entry is removed as before.
pub(super) fn bury(entry: &TableEntry, inner: &Arc<Mutex<StreamInner>>, home: &Weak<UdpSocket>) {
    let tomb = {
        let guard = inner.lock().unwrap_or_else(|e| e.into_inner());
        Tomb::from_inner(&guard, home, entry.local_id)
    };
    let Some(tomb) = tomb else { return };
    let Ok(handle) = Handle::try_current() else {
        tracing::debug!(
            local_id = entry.local_id,
            reason = "no_runtime",
            "tombstone refused"
        );
        return;
    };
    let (tx, rx) = mpsc::unbounded_channel();
    let weak = tx.downgrade();
    {
        let mut map = entry.streams.lock().unwrap_or_else(|e| e.into_inner());
        let ours = match (map.get(&entry.local_id), entry.tx.upgrade()) {
            (Some(live), Some(mine)) => live.same_channel(&mine),
            _ => false,
        };
        if !ours {
            tracing::debug!(
                local_id = entry.local_id,
                reason = "not_ours",
                "tombstone refused"
            );
            return;
        }
        map.insert(entry.local_id, tx);
    }
    tracing::debug!(
        local_id = entry.local_id,
        remote_addr = %tomb.remote_addr,
        kind = if tomb.end_acked { "Ended" } else { "Destroyed" },
        until_ms = tomb_ttl(tomb.rto).as_millis() as u64,
        "tombstone set"
    );
    // The guard removes the tombstone on any exit of the task, also when the runtime stops and drops the future.
    let guard = TableEntry {
        streams: Arc::clone(&entry.streams),
        local_id: entry.local_id,
        tx: weak,
    };
    handle.spawn(linger(tomb, rx, guard));
}

async fn linger(
    mut tomb: Tomb,
    mut rx: mpsc::UnboundedReceiver<IncomingPacket>,
    guard: TableEntry,
) {
    let mut last_reply: Option<tokio::time::Instant> = None;
    let expiry = tokio::time::sleep_until(tomb.until);
    tokio::pin!(expiry);
    loop {
        tokio::select! {
            () = &mut expiry => break,
            packet = rx.recv() => {
                let Some(packet) = packet else { break };
                let Some(udp) = tomb.udp.upgrade() else { break };
                let Ok(hdr) = Header::decode(&packet.data) else { continue };
                let (kind, buf) = match tomb.reply(&hdr, packet.addr) {
                    Reply::Silent => continue,
                    Reply::Ack(buf) => ("ack", buf),
                    Reply::Destroy(buf) => ("destroy", buf),
                };
                let now = tokio::time::Instant::now();
                if last_reply.is_some_and(|at| now.duration_since(at) < tomb.rto) {
                    continue;
                }
                last_reply = Some(now);
                tomb.sent += 1;
                tracing::debug!(
                    local_id = guard.local_id,
                    remote_addr = %tomb.remote_addr,
                    flags = hdr.type_flags,
                    reply = kind,
                    sent = tomb.sent,
                    "tombstone reply"
                );
                crate::native::tx_err::send_counted(&udp, &buf, tomb.remote_addr, "tomb").await;
            }
        }
    }
}

#[cfg(test)]
#[path = "linger_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "linger_ttl_tests.rs"]
mod ttl_tests;
