// JS: libudx/src/udx.c
//! Outgoing path: write and END preparation, sent bookkeeping, queue, assembly and sending of data packets; see also `loss.rs`, `mtu.rs`.

use super::*;

impl StreamInner {
    // ── Write preparation ────────────────────────────────────────────────────

    /// Write rejections: not connected, already closed for writing, no socket.
    pub(crate) fn check_writable(&self) -> Result<()> {
        if !self.connected && self.firewall_hook.is_none() {
            return Err(UdxError::Io(std::io::Error::other("stream not connected")));
        }
        if self.ended {
            return Err(UdxError::Io(std::io::Error::other(
                "stream already shut down",
            )));
        }
        if self.udp.is_none() {
            return Err(UdxError::StreamClosed);
        }
        Ok(())
    }

    pub(crate) fn prepare_write(&mut self, data_len: usize) -> Result<WritePrep> {
        self.check_writable()?;
        let udp = self.udp.clone().ok_or(UdxError::StreamClosed)?;
        // When the firewall hook hasn't fired yet, remote_addr is not known.
        // Use the unspecified sentinel (0.0.0.0:0); the processor will update
        // queued packets once the hook fires and remote_addr is resolved.
        let remote_addr = self
            .remote_addr
            .unwrap_or_else(|| "0.0.0.0:0".parse().unwrap());
        let remote_id = self.remote_id;

        let max_payload = self.max_payload();
        let chunk_count = if data_len == 0 {
            1
        } else {
            data_len.div_ceil(max_payload)
        };
        let first_seq = self.next_seq;
        let ack_threshold = first_seq.wrapping_add(chunk_count as u32);
        self.next_seq = ack_threshold;
        let current_ack = self.next_remote_seq;

        let (ack_tx, ack_rx) = oneshot::channel();
        self.pending_writes.push(PendingWrite {
            ack_threshold,
            tx: Some(ack_tx),
        });

        Ok(WritePrep {
            udp,
            remote_addr,
            remote_id,
            first_seq,
            current_ack,
            ack_rx,
        })
    }

    pub(crate) fn prepare_end(&mut self) -> Result<WritePrep> {
        if !self.connected {
            return Err(UdxError::Io(std::io::Error::other("stream not connected")));
        }
        if self.ended {
            return Err(UdxError::Io(std::io::Error::other(
                "stream already shut down",
            )));
        }
        self.ended = true;
        let udp = self.udp.clone().ok_or(UdxError::StreamClosed)?;
        let remote_addr = self.remote_addr.ok_or(UdxError::StreamClosed)?;
        let remote_id = self.remote_id;

        let seq = self.reserve_end_seq();
        let current_ack = self.next_remote_seq;

        let (ack_tx, ack_rx) = oneshot::channel();
        self.pending_writes.push(PendingWrite {
            ack_threshold: seq.wrapping_add(1),
            tx: Some(ack_tx),
        });

        Ok(WritePrep {
            udp,
            remote_addr,
            remote_id,
            first_seq: seq,
            current_ack,
            ack_rx,
        })
    }

    // ── Outgoing packet registration ─────────────────────────────────────────

    /// Store a sent packet in the outgoing buffer for potential retransmission.
    pub(crate) fn register_sent(&mut self, seq: u32, packet: Vec<u8>, is_mtu_probe: bool) {
        let now = Instant::now();
        let inflight = self.inflight();
        let rate_info = self.congestion.on_packet_sent(seq, inflight, now);
        tracing::trace!(
            seq,
            is_mtu_probe,
            "register_sent: packet placed in outgoing"
        );
        self.outgoing.insert(
            Seq(seq),
            SentPacket {
                packet,
                time_sent: now,
                retransmit_count: 0,
                sacked: false,
                is_mtu_probe,
                lost: false,
                rate_info: Some(rate_info),
                remote_addr: None,
            },
        );
    }

    pub(crate) fn queue_for_send(&mut self, packets: Vec<(u32, Vec<u8>)>, remote_addr: SocketAddr) {
        let count = packets.len();
        for (seq, packet) in packets {
            self.send_queue.push_back(QueuedPacket {
                packet,
                seq,
                remote_addr,
            });
        }
        tracing::trace!(
            count,
            queue_len = self.send_queue.len(),
            "queue_for_send: packets staged"
        );
        if let Some(ref tx) = self.notify_tx {
            let _ = tx.send(StreamNotify::DataQueued);
        }
    }
}

// ── Packet building ──────────────────────────────────────────────────────────

/// Build a wire-format DATA packet (header + payload).
pub(crate) fn build_data_packet(remote_id: u32, seq: u32, ack: u32, payload: &[u8]) -> Vec<u8> {
    let header = Header {
        type_flags: FLAG_DATA,
        data_offset: 0,
        remote_id,
        recv_window: DEFAULT_RWND,
        seq,
        ack,
    };
    let mut packet = Vec::with_capacity(HEADER_SIZE + payload.len());
    packet.extend_from_slice(&header.encode());
    packet.extend_from_slice(payload);
    packet
}

// ── send_data_packets ────────────────────────────────────────────────────────

/// Build, register, and send DATA packets for a write.
///
/// Packets are stored in the outgoing buffer *before* being sent on the wire,
/// ensuring the retransmission engine can resend them if ACKs don't arrive.
pub(crate) async fn send_data_packets(
    inner: &Arc<Mutex<StreamInner>>,
    _udp: &tokio::net::UdpSocket,
    remote_addr: SocketAddr,
    remote_id: u32,
    first_seq: u32,
    current_ack: u32,
    data: &[u8],
) -> Result<()> {
    let max_payload = inner
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .max_payload();
    let chunks: Vec<&[u8]> = if data.is_empty() {
        vec![&[]]
    } else {
        data.chunks(max_payload).collect()
    };

    let mut packets: Vec<(u32, Vec<u8>)> = Vec::with_capacity(chunks.len());
    for (i, chunk) in chunks.iter().enumerate() {
        let seq = first_seq.wrapping_add(i as u32);
        packets.push((seq, build_data_packet(remote_id, seq, current_ack, chunk)));
    }

    {
        let mut guard = inner.lock().unwrap_or_else(|e| e.into_inner());
        if guard.outgoing.is_empty() && guard.send_queue.is_empty() {
            guard
                .congestion
                .on_transmit_start(std::time::Instant::now());
        }
        guard.queue_for_send(packets, remote_addr);
    }
    Ok(())
}
