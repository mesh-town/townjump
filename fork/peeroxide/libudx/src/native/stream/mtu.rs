// JS: libudx/src/udx.c
//! MTU search: `MtuState`, probe, probe ack, padding removal; related — `inner.rs`, `outgoing.rs`.

use super::*;

// ── MTU Probing State ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MtuState {
    Base,
    Search,
    SearchComplete,
}

impl StreamInner {
    /// Inflate a DATA packet to `mtu_probe_size` by inserting zero padding after the header.
    /// Sets `data_offset` so the receiver skips the padding via `payload_offset()`.
    /// Returns `true` if the packet was successfully probeified.
    pub(super) fn mtu_probeify_packet(&mut self, packet: &mut Vec<u8>) -> bool {
        if packet.len() < HEADER_SIZE {
            return false;
        }
        // The probe target is the IP packet size (udx.c:313-316).
        let target_size = self.mtu_probe_size;
        let ip_udp_overhead = self.ip_overhead() - HEADER_SIZE;
        let ip_packet_size = packet.len() + ip_udp_overhead;
        if target_size <= ip_packet_size {
            return false;
        }

        let padding = target_size - ip_packet_size;
        // data_offset is u8 — can only pad up to 255 bytes.
        // This naturally limits probing to nearly-full packets (matching C libudx).
        if padding > 255 {
            return false;
        }
        // Insert padding between header and payload: shift payload right
        let payload = packet[HEADER_SIZE..].to_vec();
        packet.truncate(HEADER_SIZE);
        packet.extend(std::iter::repeat_n(0u8, padding));
        packet.extend_from_slice(&payload);

        // Set data_offset in header byte [3]
        packet[3] = padding as u8;

        self.mtu_probe_wanted = false;
        tracing::trace!(probe_size = target_size, padding, "mtu: probeify");
        true
    }

    /// Clamp of the ladder top by the link MTU to the remote address — the reference's table (udx.c:2593-2602).
    /// We do not touch `mtu_state` above `MTU_BASE`: the stream may already be in `Search`.
    pub(crate) fn apply_link_mtu(&mut self, link_mtu: Option<usize>) {
        let mtu = match link_mtu {
            Some(m) if m <= MTU_MAX => m,
            _ => MTU_MAX,
        };
        if link_mtu.is_some() && mtu <= MTU_BASE {
            self.mtu_state = MtuState::SearchComplete;
        }
        // For mtu <= MTU_BASE the reference puts the value itself into mtu_max and does not lower `mtu` (it stays MTU_BASE), so
        // mtu_max < mtu is a property of the reference, not an oversight.
        self.mtu_max = mtu;
    }

    pub(super) fn on_mtu_probe_acked(&mut self) {
        let old_mtu = self.mtu;
        self.mtu = self.mtu_probe_size;
        self.mtu_probe_count = 0;
        self.mtu_probe_size += MTU_STEP;

        if self.mtu_probe_size > self.mtu_max {
            self.mtu_probe_size = self.mtu_max;
        }

        if self.mtu >= self.mtu_max {
            self.mtu_state = MtuState::SearchComplete;
        } else {
            self.mtu_probe_wanted = true;
        }

        let new_mss = self.max_payload();
        self.congestion.on_mtu_change(old_mtu, self.mtu, new_mss);
        tracing::debug!(old_mtu, new_mtu = self.mtu, state = ?self.mtu_state, "mtu: probe acked");
    }
}

/// Strip MTU probe padding from a packet: remove zero bytes between header and payload,
/// reset data_offset to 0.
pub(super) fn strip_probe_padding(packet: &mut Vec<u8>) {
    if packet.len() < HEADER_SIZE {
        return;
    }
    let data_offset = packet[3] as usize;
    if data_offset == 0 {
        return;
    }
    let payload_start = HEADER_SIZE + data_offset;
    if payload_start <= packet.len() {
        let payload = packet[payload_start..].to_vec();
        packet.truncate(HEADER_SIZE);
        packet.extend_from_slice(&payload);
    }
    packet[3] = 0;
}
