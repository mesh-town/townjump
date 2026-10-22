// JS: libudx/src/udx.c:send_packets, retransmit_packet, stream_may_send (retransmission)
//! Retransmissions through the window and pacing gates: the lost go ahead of new packets (C `send_packets`, 929-943).

use super::*;
use std::time::Duration;

impl StreamInner {
    /// Retransmissions into `out` while the gates allow. `Ok` — nothing lost; `Err` — as `send_gate`: `None` window, `Some(d)` pacing.
    pub(super) fn send_retransmits(
        &mut self,
        out: &mut Vec<(Vec<u8>, SocketAddr)>,
    ) -> std::result::Result<(), Option<Duration>> {
        let Some(addr) = self.remote_addr else {
            return Ok(());
        };
        let mut cursor = None;
        while let Some(seq) = self.outgoing.first_lost(cursor) {
            self.retransmit_gate()?;
            cursor = Some(seq);
            self.retransmit_one(seq, addr, out);
        }
        Ok(())
    }

    /// Retransmission gate (C `stream_may_send`, 632-646): pacing, then `inflight < cwnd`; the receive window does not hold a retransmission.
    fn retransmit_gate(&mut self) -> std::result::Result<(), Option<Duration>> {
        let delay = self.congestion.pacing_delay(0, Instant::now());
        if delay > Duration::ZERO {
            return Err(Some(delay));
        }
        if self.inflight() >= self.congestion.cwnd {
            return Err(None);
        }
        Ok(())
    }

    /// One retransmission into `out`, pacing tokens debited (C `retransmit_packet` → `_send_packet`, 910-927, 663-682).
    fn retransmit_one(&mut self, seq: Seq, addr: SocketAddr, out: &mut Vec<(Vec<u8>, SocketAddr)>) {
        let route = self.outgoing.get(&seq).and_then(|p| p.remote_addr);
        let Some(packet) = self.resend_seq(seq, Instant::now()) else {
            return;
        };
        self.congestion
            .pacing
            .try_consume(packet.len() as u32, Instant::now());
        out.push((packet, route.unwrap_or(addr)));
    }

    /// Retransmits a number; a retransmitted MTU probe is a search failure. `None` — the number is not lost.
    pub(super) fn resend_seq(&mut self, seq: Seq, now: Instant) -> Option<Vec<u8>> {
        let (packet, was_probe) = self.outgoing.resend(seq, now)?;
        // C `_send_packet` calls `udx__rate_pkt_sent` for a retransmission too; a retransmission does not restart the flight (`udx_rate.c:8-15`).
        let info = self.congestion.on_packet_resent(now);
        self.outgoing.set_rate_info(seq, info);
        self.note_probe_resent(was_probe);
        Some(packet)
    }

    fn note_probe_resent(&mut self, was_probe: bool) {
        if !was_probe {
            return;
        }
        self.on_probe_resent("retransmit");
    }

    /// There is something to send: new data in the queue or waiting for retransmission (kick after ACK, RTO, REO).
    pub(super) fn has_pending_send(&self) -> bool {
        !self.send_queue.is_empty() || self.outgoing.lost() > 0
    }
}

#[cfg(test)]
#[path = "retransmit_tests.rs"]
mod tests;
