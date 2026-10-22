// JS: dht-rpc/lib/io.js
//! `impl Io`: receive loop, drain, timeout deadline and timeout checks.
//! Neighbours: `datagram` handles what `recv` reads; `requests` resends.

use std::sync::atomic::Ordering;
use std::time::Duration;

use blake2::digest::Mac;
use tokio::time::Instant;

use super::state::Io;
use super::types::{Blake2bMac256, IoEvent, RetryEvent, SocketKind, TimeoutEvent};

impl Io {
    /// Receive and decode the next message from either socket.
    /// Returns None only if both channels are closed.
    pub async fn recv(&mut self) -> Option<IoEvent> {
        loop {
            let (datagram, socket_kind) = tokio::select! {
                biased;
                msg = self.client_rx.recv() => (msg?, SocketKind::Client),
                msg = self.server_rx.recv() => (msg?, SocketKind::Server),
            };
            self.wire
                .bytes_received
                .fetch_add(datagram.data.len() as u64, Ordering::Relaxed);
            tracing::trace!(
                from = %datagram.addr,
                len = datagram.data.len(),
                first_byte = datagram.data.first().copied().unwrap_or(0),
                ?socket_kind,
                "IO::recv raw datagram"
            );
            if let Some(event) = self.process_datagram(datagram, socket_kind) {
                return Some(event);
            }
        }
    }

    /// Drain congestion window and rotate secrets. Call every ~750 ms.
    pub fn drain(&mut self) {
        if let Some(secrets) = &mut self.secrets {
            self.rotate_countdown -= 1;
            if self.rotate_countdown == 0 {
                self.rotate_countdown = 10;
                // Rotate: swap[0] and [1], then re-hash old [0] (now at [1]).
                secrets.swap(0, 1);
                // Hash secrets[1] (the old secrets[0]) with itself.
                if let Ok(mut mac) = Blake2bMac256::new_from_slice(&secrets[1]) {
                    mac.update(&secrets[1]);
                    let hash = mac.finalize().into_bytes();
                    secrets[1].copy_from_slice(&hash);
                }
            }
        }

        self.congestion.drain();

        while !self.congestion.is_full() {
            match self.pending.pop_front() {
                Some(p) => {
                    if let Some(idx) = self.inflight.iter().position(|e| e.tid == p.tid) {
                        self.send_inflight_at(idx);
                    }
                }
                None => break,
            }
        }
    }

    /// Return the earliest deadline across all inflight requests.
    /// Returns a far-future instant if there are no inflight requests.
    pub fn next_timeout_deadline(&self) -> Instant {
        self.inflight
            .iter()
            .map(|e| e.deadline)
            .min()
            .unwrap_or_else(|| Instant::now() + Duration::from_secs(3600))
    }

    /// Check for expired timeouts. Returns events for timed-out requests.
    /// Performs retransmissions silently; retransmission events come from `check_timeouts_and_retries`.
    pub fn check_timeouts(&mut self) -> Vec<TimeoutEvent> {
        self.check_timeouts_and_retries().1
    }

    /// Returns events for intermediate retransmissions and final timeouts; an entry
    /// never sent yet (congestion queue) produces no retransmission event.
    pub(crate) fn check_timeouts_and_retries(&mut self) -> (Vec<RetryEvent>, Vec<TimeoutEvent>) {
        let now = Instant::now();
        let mut timeout_indices: Vec<usize> = Vec::new();
        let mut retry_indices: Vec<usize> = Vec::new();

        for (i, entry) in self.inflight.iter().enumerate() {
            if entry.deadline <= now {
                if entry.sent > entry.retries {
                    timeout_indices.push(i);
                } else {
                    retry_indices.push(i);
                }
            }
        }

        // Process retries first (in-place, no index shifts).
        let mut retries = Vec::with_capacity(retry_indices.len());
        for &i in &retry_indices {
            self.stats.retries += 1;
            let was_sent = self.inflight[i].sent >= 1;
            self.send_inflight_at(i);
            if was_sent {
                let entry = &self.inflight[i];
                retries.push(RetryEvent {
                    tid: entry.tid,
                    to: entry.to.clone(),
                });
            }
        }

        // Remove timed-out entries from highest to lowest index.
        timeout_indices.sort_unstable_by(|a, b| b.cmp(a));
        let mut events = Vec::with_capacity(timeout_indices.len());
        for i in timeout_indices {
            let entry = self.inflight.swap_remove(i);
            self.congestion.recv();
            self.stats.active = self.stats.active.saturating_sub(1);
            self.stats.timeouts += 1;
            events.push(TimeoutEvent {
                tid: entry.tid,
                to: entry.to,
                command: entry.command,
                internal: entry.internal,
                target: entry.target,
                node_id: entry.node_id,
            });
        }
        (retries, events)
    }
}
