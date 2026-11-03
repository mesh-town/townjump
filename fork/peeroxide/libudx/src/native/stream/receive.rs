// JS: libudx/src/udx.c
//! Data receive: ordering, SACK ranges and read event delivery; see also `incoming.rs`, `loss.rs`.

use std::ops::ControlFlow;

use super::*;

/// DATA / END processing (unified, handles combined DATA+END packets): ACK for every DATA/END, and for a duplicate (D8).
///
/// `Break` means the stream is locally closed and the peer sent new payload: the packet is neither delivered nor
/// acknowledged, the caller answers with DESTROY and ends the processor. Duplicates and END without payload are
/// acknowledged as before.
pub(super) fn handle_data_packet(
    inner: &Mutex<StreamInner>,
    header: &Header,
    data: &[u8],
) -> ControlFlow<()> {
    let is_data = header.has_flag(FLAG_DATA);
    let is_end = header.has_flag(FLAG_END);
    if !(is_data || is_end) {
        return ControlFlow::Continue(());
    }
    let payload_start = header.payload_offset();
    let payload = if payload_start <= data.len() {
        data[payload_start..].to_vec()
    } else {
        Vec::new()
    };
    let recv_flags = (if is_data { FLAG_DATA } else { 0 }) | (if is_end { FLAG_END } else { 0 });

    let (events, read_tx) = {
        let mut guard = inner.lock().unwrap_or_else(|e| e.into_inner());
        if guard.read_closed
            && is_data
            && !payload.is_empty()
            && seq_diff(header.seq, guard.next_remote_seq) >= 0
        {
            return ControlFlow::Break(());
        }
        let events = guard.receive_data(header.seq, payload, recv_flags);
        send_ack(&guard);
        (events, guard.read_tx.clone())
    };
    for event in events {
        let len = match &event {
            StreamEvent::Data(data) => data.len(),
            StreamEvent::End => 0,
        };
        if read_tx.send(event).is_err() && len > 0 {
            // the reader is gone: nobody will release these bytes
            inner
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .release_read(len);
        }
    }
    ControlFlow::Continue(())
}

impl StreamInner {
    // ── Receive buffering ────────────────────────────────────────────────────

    /// Handle an incoming DATA or END packet.
    ///
    /// In-order packets are delivered immediately. Out-of-order packets are
    /// buffered. When gaps are filled, consecutive buffered packets are drained
    /// and delivered in order. Returns events to deliver to the application.
    ///
    /// A single packet may carry both FLAG_DATA and FLAG_END (C libudx combines
    /// the last data chunk with the end marker). In that case two events are
    /// produced: Data followed by End.
    pub(super) fn receive_data(
        &mut self,
        seq: u32,
        payload: Vec<u8>,
        flags: u8,
    ) -> Vec<StreamEvent> {
        let mut events = Vec::new();

        // Duplicate or already-received packet — ignore
        if seq_diff(seq, self.next_remote_seq) < 0 {
            return events;
        }

        if seq == self.next_remote_seq {
            // In-order: deliver directly (always taken, a retransmission ignores the window)
            push_events(&mut events, payload, flags);
            self.next_remote_seq = seq.wrapping_add(1);

            // Drain consecutive buffered packets
            while let Some(recvd) = self.recv_buf.remove(&Seq(self.next_remote_seq)) {
                self.recv_buf_bytes -= recvd.payload.len();
                push_events(&mut events, recvd.payload, recvd.flags);
                self.next_remote_seq = self.next_remote_seq.wrapping_add(1);
            }
            for event in &events {
                if let StreamEvent::Data(data) = event {
                    self.note_unread(data.len());
                }
            }
        } else if self.ooo_fits(payload.len()) {
            // Out-of-order: buffer for later delivery
            if let std::collections::btree_map::Entry::Vacant(slot) = self.recv_buf.entry(Seq(seq))
            {
                self.recv_buf_bytes += payload.len();
                slot.insert(RecvdPacket { payload, flags });
            }
        }

        self.remote_ended |= events.iter().any(|e| matches!(e, StreamEvent::End));
        events
    }

    // ── SACK generation ──────────────────────────────────────────────────────

    /// Compute SACK ranges from the out-of-order receive buffer.
    /// Returns contiguous ranges of received sequence numbers that are
    /// beyond the cumulative ACK point.
    pub(super) fn sack_ranges(&self) -> Vec<SackRange> {
        if self.recv_buf.is_empty() {
            return Vec::new();
        }

        let mut ranges = Vec::new();
        let mut iter = self.recv_buf.keys();

        if let Some(&Seq(first)) = iter.next() {
            let mut start = first;
            let mut end = first.wrapping_add(1);

            for &Seq(seq) in iter {
                if seq == end {
                    // Extend current range
                    end = seq.wrapping_add(1);
                } else {
                    // Gap found: close current range, start new one
                    ranges.push(SackRange { start, end });
                    if ranges.len() >= MAX_SACK_RANGES {
                        return ranges;
                    }
                    start = seq;
                    end = seq.wrapping_add(1);
                }
            }
            ranges.push(SackRange { start, end });
        }
        ranges
    }
}

/// Push stream events for a received packet. When both FLAG_DATA and FLAG_END
/// are set (combined packet), pushes Data then End.
fn push_events(events: &mut Vec<StreamEvent>, payload: Vec<u8>, flags: u8) {
    let has_data = flags & FLAG_DATA != 0;
    let has_end = flags & FLAG_END != 0;

    if has_data && !payload.is_empty() {
        events.push(StreamEvent::Data(payload));
    } else if has_data {
        // Empty data packet (e.g., keepalive DATA) — skip
    } else if !has_end {
        // Neither DATA nor END — shouldn't happen, but guard
        return;
    }

    if has_end {
        events.push(StreamEvent::End);
    }
}
