//! ACK and HEARTBEAT packets and their sending. Format as in JS libudx (udx.c: send_ack, send_heartbeat).
use super::*;
use crate::native::header::encode_sack;

/// ACK: 20 header bytes + up to `MAX_SACK_RANGES` SACK blocks right after it (`data_offset` = 0, as in JS).
pub(super) fn ack_packet(remote_id: u32, seq: u32, ack: u32, sacks: &[SackRange]) -> Vec<u8> {
    let sacks = &sacks[..sacks.len().min(MAX_SACK_RANGES)];
    let type_flags = if sacks.is_empty() { 0 } else { FLAG_SACK };
    let mut packet = header_bytes(type_flags, remote_id, seq, ack);
    let mut blocks = vec![0u8; sacks.len() * 8];
    encode_sack(sacks, &mut blocks);
    packet.extend_from_slice(&blocks);
    packet
}

/// HEARTBEAT: 20 header bytes, no payload (the reply to HEARTBEAT is an ACK; the probe itself is ZWP).
pub(super) fn heartbeat_packet(remote_id: u32, seq: u32, ack: u32) -> Vec<u8> {
    header_bytes(FLAG_HEARTBEAT, remote_id, seq, ack)
}

/// MESSAGE: 20 header bytes and the payload; outside the sequence space (JS `udx_stream_send_message`).
pub(super) fn message_packet(remote_id: u32, seq: u32, ack: u32, payload: &[u8]) -> Vec<u8> {
    let mut packet = header_bytes(FLAG_MESSAGE, remote_id, seq, ack);
    packet.extend_from_slice(payload);
    packet
}

fn header_bytes(type_flags: u8, remote_id: u32, seq: u32, ack: u32) -> Vec<u8> {
    Header {
        type_flags,
        data_offset: 0,
        remote_id,
        recv_window: DEFAULT_RWND,
        seq,
        ack,
    }
    .encode()
    .to_vec()
}

/// Send "in the background": without a socket or address — nothing.
pub(super) fn send_raw(inner: &StreamInner, packet: Vec<u8>) {
    let (Some(udp), Some(addr)) = (inner.udp.clone(), inner.remote_addr) else {
        return;
    };
    tokio::spawn(async move {
        let _sent = crate::native::tx_err::send_counted(&udp, &packet, addr, "raw").await;
    });
}

/// ACK with the current SACK blocks (JS `send_ack`).
pub(super) fn send_ack(inner: &StreamInner) {
    let packet = ack_packet(
        inner.remote_id,
        inner.next_send_seq(),
        inner.next_remote_seq,
        &inner.sack_ranges(),
    );
    send_raw(inner, packet);
}

impl StreamInner {
    /// JS `stream->seq`: seq of the first unsent packet.
    pub(super) fn next_send_seq(&self) -> u32 {
        self.send_queue.front().map_or(self.next_seq, |q| q.seq)
    }
}

#[cfg(test)]
#[path = "wire_tests.rs"]
mod tests;
