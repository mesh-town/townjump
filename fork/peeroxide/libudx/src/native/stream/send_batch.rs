// JS: libudx/src/udx.c (send_packets, stream_may_send)
//! One sender wakeup: from `send_queue` to `outgoing` while the window and pacing allow.
//! Calls `send_step` (`send_step.rs`); the socket is there too, outside the lock.

use super::*;
use std::time::Duration;

/// Wakeup result: packets for the socket in order and the pacing delay the loop stopped on (`None` — it did not stop on it).
pub(super) struct SendBatch {
    pub(super) packets: Vec<(Vec<u8>, SocketAddr)>,
    pub(super) pacing_delay: Option<Duration>,
    /// The first `rtx` packets are retransmissions (Z diagnostics).
    pub(super) rtx: usize,
}

impl StreamInner {
    /// Retransmissions first, then new ones — everything the window and pacing allow.
    pub(super) fn take_send_batch(&mut self) -> SendBatch {
        self.take_send_batch_with(fix::mode().pace)
    }

    /// `pace` is on by default (`UDX_FIX=legacy` turns it off): the pacer is refilled on every attempt before the window check.
    pub(super) fn take_send_batch_with(&mut self, pace: bool) -> SendBatch {
        self.debug_check_counters();
        let mut batch = SendBatch {
            packets: Vec::new(),
            pacing_delay: None,
            rtx: 0,
        };
        let rtx_result = self.send_retransmits(&mut batch.packets);
        batch.rtx = batch.packets.len();
        if let Err(delay) = rtx_result {
            batch.pacing_delay = delay;
            return batch;
        }
        loop {
            if let Err(delay) = self.send_gate(pace) {
                batch.pacing_delay = delay;
                break;
            }
            let Some(queued) = self.send_queue.pop_front() else {
                break;
            };
            let sent = self.send_one(queued);
            batch.packets.push(sent);
        }
        batch
    }

    /// Ok — the queue head can be sent; Err(None) — the queue is empty or the window is full; Err(Some(d)) — wait for pacing `d`.
    fn send_gate(&mut self, pace: bool) -> std::result::Result<(), Option<Duration>> {
        let Some(front) = self.send_queue.front() else {
            return Err(None);
        };
        // Not connected yet (the firewall hook has not fired): like C `stream_may_send` (needs CONNECTED) hold the head. Sent to the 0.0.0.0:0
        // stub it would be lost and only the RTO would resend it (+1 s); the hook readdresses the queue and wakes the sender.
        if front.remote_addr == upgrade::stub_addr() {
            return Err(None);
        }
        let packet_len = front.packet.len() as u32;
        // C `stream_may_send` refills the pacer first, at any window (udx.c:632-646).
        if pace {
            self.congestion.pacing.refill(Instant::now());
        }
        if !self.window_open() {
            return Err(None);
        }
        let delay = self.congestion.pacing_delay(packet_len, Instant::now());
        if delay > Duration::ZERO {
            return Err(Some(delay));
        }
        Ok(())
    }

    /// Z: step result into the `ZS` roll-up. Gate closing reason: pacing set — pace; otherwise window closed — win; otherwise queue.
    pub(super) fn diag_step(&mut self, sent: usize, rtx: usize, pacing_delay: Option<Duration>) {
        let why = if pacing_delay.is_some() {
            diag::Why::Pace
        } else if !self.window_open() {
            diag::Why::Win
        } else {
            diag::Why::Queue
        };
        diag::step(&diag::Step {
            sent_new: sent.saturating_sub(rtx) as u32,
            sent_rtx: rtx as u32,
            pace_delay_us: pacing_delay.map_or(0, |d| d.as_micros() as u64),
            rate: self.congestion.pacing_bytes_per_ms,
            why,
            cwnd: self.congestion.cwnd,
            inflight: self.inflight(),
            lost: self.outgoing.lost(),
            sacked: self.outgoing.sacked(),
            ca_state: self.congestion.ca_state as u8,
        });
        if let Some(t) = self.zp.due(Instant::now()) {
            eprintln!("{}", self.zp.line(t, &self.zp_view()));
        }
    }

    /// The window allows one more packet; before the check it marks app-limited (C `udx__rate_check_app_limited`).
    fn window_open(&mut self) -> bool {
        let inflight = self.inflight();
        // "Nothing to send" as in C (`writes_queued_bytes`, udx.c:1733): queue plus sent-without-SACK; C subtracts the SACKed.
        // A retransmission goes before new ones and locks them out while it waits for the gate (`retransmit.rs`); the waiting retransmission is already
        // in `unsacked_bytes`, so `retransmit_pending` in `check_app_limited` is always `false` here.
        let queued_bytes = self.send_queue.bytes() + self.outgoing.unsacked_bytes();
        self.congestion.check_app_limited(queued_bytes, inflight);
        self.congestion.can_send(inflight, self.send_rwnd)
    }

    /// The detached queue head: if needed an MTU probe, then into `outgoing` and pacing debit. Result — a packet and an address for the socket.
    fn send_one(&mut self, queued: QueuedPacket) -> (Vec<u8>, SocketAddr) {
        let mut packet = queued.packet;
        let is_mtu_probe = self.mtu_probe_for(&mut packet);
        self.register_sent(queued.seq, packet.clone(), is_mtu_probe);
        self.note_route(queued.seq, queued.remote_addr);
        self.congestion
            .pacing
            .try_consume(packet.len() as u32, Instant::now());
        (packet, queued.remote_addr)
    }

    /// The first DATA starts the MTU search; during the search the packet becomes a probe if `mtu_probeify_packet` padded it.
    fn mtu_probe_for(&mut self, packet: &mut Vec<u8>) -> bool {
        if self.change.is_some() {
            return false; // no probes while the route changes (udx.c:803)
        }
        let is_data = packet.len() >= HEADER_SIZE && packet[2] & FLAG_DATA != 0;
        if is_data && self.mtu_state == MtuState::Base {
            self.mtu_state = MtuState::Search;
            self.mtu_probe_wanted = true;
        }
        is_data
            && self.mtu_state == MtuState::Search
            && self.mtu_probe_wanted
            && self.mtu_probeify_packet(packet)
    }
}

#[cfg(test)]
#[path = "send_batch_tests.rs"]
mod tests;
