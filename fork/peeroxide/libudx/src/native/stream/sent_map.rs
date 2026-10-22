// JS: libudx/src/udx.c (inflight_queue.len, ack_packet)
//! Map of sent packets `outgoing` with SACK and loss counters: the `sacked`/`lost` flags change only here,
//! so `inflight` is O(1). See also `loss.rs` (losses, retransmission), `sack.rs` (SACK), `outgoing.rs` (`register_sent`).
//! The send-order index for RACK is `sent_map_rack.rs`; the indexes of numbers without SACK and of lost ones are `sent_map_index.rs`
//! (ACK cost scales with the number of marked entries, not the map size, like the C lists).

use super::rack::Rack;
use super::*;
use crate::native::congestion::rate::PacketRateInfo;
use std::ops::Deref;
use std::time::Duration;

#[path = "sent_map_rack.rs"]
mod rack_walk;
use rack_walk::SendOrder;

#[path = "sent_map_index.rs"]
mod seq_index;
use seq_index::SeqIndex;

/// Sent packets not yet confirmed by a cumulative ACK. Read as a `BTreeMap` (via `Deref`);
/// changed only through the methods below, there is no `DerefMut` — a packet flag cannot be changed bypassing the counters.
#[derive(Default)]
pub(super) struct SentMap {
    map: BTreeMap<Seq, SentPacket>,
    /// Packets with `sacked`.
    sacked: u32,
    /// Packets with `lost`: set by `mark_rack_lost`/`mark_rto_lost`, cleared by retransmission or SACK.
    lost: u32,
    /// Highest number among `sacked` (`Seq` order, with wraparound); `None` when `sacked == 0`.
    highest_sacked: Option<Seq>,
    /// RACK clock (C rack_time_sent, rack_rtt), see rack.rs. Monotonic; a retransmission moves it if its RTT is not shorter than `rtt_min`.
    rack: Rack,
    /// Diagnostics: cumulative count of packets retransmitted via `resend_one` (RTO and fast retransmit).
    resent: u32,
    /// Sum of `packet.len()` (with header) of entries without SACK: the "nothing to send" input for app-limited, like `writes_queued_bytes` in C.
    unsacked_bytes: usize,
    /// Send-order index of entries without SACK and loss (RACK walks it, as C walks `inflight_queue`).
    order: SendOrder,
    /// Numbers of entries without SACK (including lost: they can still be SACKed): `on_sack` walks them, not the block.
    unsacked: SeqIndex,
    /// Numbers of entries with `lost`: `first_lost` is O(log n).
    lost_seqs: SeqIndex,
    /// Z diagnostics: send-order index entries up to `rack.sent` walked in the last `mark_rack_lost`.
    pub(super) rack_visited: u32,
}

impl Deref for SentMap {
    type Target = BTreeMap<Seq, SentPacket>;

    fn deref(&self) -> &Self::Target {
        &self.map
    }
}

impl SentMap {
    /// Tests: an empty map; `rack_index = false` means no send-order index (the by-number reference).
    #[cfg(test)]
    pub(super) fn new(rack_index: bool) -> Self {
        let order = if rack_index {
            SendOrder::default()
        } else {
            SendOrder::disabled()
        };
        Self {
            order,
            ..Self::default()
        }
    }

    /// In flight: without SACK and without loss (C `inflight_queue.len`); `sacked && lost` never happens — see `mark_sacked`.
    pub(super) fn inflight(&self) -> u32 {
        self.map.len() as u32 - self.sacked - self.lost
    }

    /// Number of packets with SACK (C `stream->sacks`).
    pub(super) fn sacked(&self) -> u32 {
        self.sacked
    }

    /// RACK clock (time and RTT of the last delivery, min RTT).
    pub(super) fn rack(&self) -> &Rack {
        &self.rack
    }

    /// Tests: direct access to the RACK state.
    #[cfg(test)]
    pub(super) fn rack_mut(&mut self) -> &mut Rack {
        &mut self.rack
    }

    /// Stores a sent packet. A number in flight does not repeat (window ≪ 2^31): a repeat panics in debug.
    pub(super) fn insert(&mut self, seq: Seq, pkt: SentPacket) {
        debug_assert!(
            !(pkt.sacked && pkt.lost),
            "packet {seq:?}: SACK and loss together"
        );
        self.count(seq, &pkt);
        let (time_sent, in_flight) = (pkt.time_sent, !pkt.sacked && !pkt.lost);
        let (sacked, lost) = (pkt.sacked, pkt.lost);
        let old = self.map.insert(seq, pkt);
        debug_assert!(old.is_none(), "number {seq:?} already in flight");
        if let Some(old) = old {
            self.uncount(&old);
            if !old.sacked && !old.lost {
                self.order.remove(seq, old.time_sent);
            }
            if !old.sacked {
                self.unsacked.remove(seq);
            }
            if old.lost {
                self.lost_seqs.remove(seq);
            }
        }
        if in_flight {
            self.order.add(seq, time_sent);
        }
        if !sacked {
            self.unsacked.add(seq);
        }
        if lost {
            self.lost_seqs.add(seq);
        }
    }

    /// Numbers without SACK in `[start, end)` ascending (empty when `start >= end`).
    pub(super) fn unsacked_in(&self, start: Seq, end: Seq) -> Vec<Seq> {
        self.unsacked.range_vec(start, end)
    }

    /// SACK of packet `seq`. `Some` — the packet is in the map and marked for the first time; its loss is cleared
    /// (nobody reads the `lost` flag of a SACKed packet, so the invariant holds without conditions).
    pub(super) fn mark_sacked(&mut self, seq: Seq, now: Instant) -> Option<&SentPacket> {
        let pkt = self.map.get_mut(&seq).filter(|p| !p.sacked)?;
        if !pkt.lost {
            self.order.remove(seq, pkt.time_sent);
        } else {
            self.lost_seqs.remove(seq);
        }
        self.unsacked.remove(seq);
        pkt.sacked = true;
        self.sacked += 1;
        self.unsacked_bytes -= pkt.packet.len();
        self.lost -= u32::from(std::mem::take(&mut pkt.lost));
        self.highest_sacked = self.highest_sacked.max(Some(seq));
        self.rack.on_acked_seq(seq, pkt.retransmit_count > 0);
        self.rack
            .on_delivered(pkt.time_sent, pkt.retransmit_count > 0, now);
        Some(&*pkt)
    }

    /// Removes packets with numbers before `ack` (`Seq` order) and subtracts them from the counters; result — the removed ones ascending.
    pub(super) fn remove_before(&mut self, ack: Seq, now: Instant) -> BTreeMap<Seq, SentPacket> {
        let kept = self.map.split_off(&ack);
        let removed = std::mem::replace(&mut self.map, kept);
        removed.values().for_each(|pkt| self.uncount(pkt));
        for (&seq, pkt) in removed.iter().filter(|(_, p)| !p.sacked && !p.lost) {
            self.order.remove(seq, pkt.time_sent);
        }
        self.unsacked.split_before(ack);
        self.lost_seqs.split_before(ack);
        // C `ack_packet` moves RACK on a cumulative ACK too; a SACKed packet is already delivered (waiting for a hole is not RTT).
        let delivered = removed.iter().filter(|(_, p)| !p.sacked);
        delivered.for_each(|(&seq, p)| {
            self.rack.on_acked_seq(seq, p.retransmit_count > 0);
            self.rack
                .on_delivered(p.time_sent, p.retransmit_count > 0, now)
        });
        removed
    }

    /// Tests: overrides the send time of packet `seq` (deterministic send order without sleeping).
    #[cfg(test)]
    pub(super) fn set_time_sent(&mut self, seq: Seq, at: Instant) {
        if let Some(pkt) = self.map.get_mut(&seq) {
            if !pkt.sacked && !pkt.lost {
                self.order.remove(seq, pkt.time_sent);
                self.order.add(seq, at);
            }
            pkt.time_sent = at;
        }
    }

    /// Bytes in the map without SACK (with headers): awaiting ack and retransmission; SACKed ones do not count (C `on_bytes_acked`).
    pub(super) fn unsacked_bytes(&self) -> usize {
        self.unsacked_bytes
    }

    /// Cumulative number of retransmitted packets (`stats()` diagnostics).
    pub(super) fn resent(&self) -> u32 {
        self.resent
    }

    /// RTO marking (C 1203-1228): the head and all with `time_sent + rack.rtt + reo_wnd < now`, without SACK and not yet lost
    /// (lost ones already wait for retransmission, C 1207-1209). Result — marked numbers ascending.
    pub(super) fn mark_rto_lost(&mut self, reo_wnd: Duration, now: Instant) -> Vec<Seq> {
        let wait = self.rack.rtt + reo_wnd;
        let head = self.map.keys().next().copied();
        let lost: Vec<Seq> = self
            .map
            .iter_mut()
            .filter(|(seq, p)| {
                !p.sacked && !p.lost && (Some(**seq) == head || p.time_sent + wait < now)
            })
            .map(|(&seq, p)| {
                p.lost = true;
                self.order.remove(seq, p.time_sent);
                seq
            })
            .collect();
        lost.iter().for_each(|&seq| self.lost_seqs.add(seq));
        self.lost += lost.len() as u32;
        lost
    }

    /// Packets awaiting retransmission (C `retransmit_queue.len`).
    pub(super) fn lost(&self) -> u32 {
        self.lost
    }

    /// The first number awaiting retransmission after `after` (`None` — from the head), ascending.
    pub(super) fn first_lost(&self, after: Option<Seq>) -> Option<Seq> {
        self.lost_seqs.first_after(after)
    }

    /// Retransmission of lost `seq` (C `_send_packet` for a retransmission): loss cleared, packet in flight, `resent` += 1.
    /// Result — a copy and "was an MTU probe"; `None` — no such number or it is not lost.
    pub(super) fn resend(&mut self, seq: Seq, now: Instant) -> Option<(Vec<u8>, bool)> {
        let pkt = self.map.get_mut(&seq).filter(|p| p.lost)?;
        pkt.lost = false;
        self.lost -= 1;
        self.lost_seqs.remove(seq);
        let before = pkt.packet.len();
        let out = resend_one(pkt, now);
        self.unsacked_bytes -= before - out.0.len();
        self.resent = self.resent.wrapping_add(1);
        self.order.add(seq, now);
        Some(out)
    }

    /// New packet rate snapshot on retransmission (C `udx__rate_pkt_sent` in `_send_packet`); does not touch the counters.
    pub(super) fn set_rate_info(&mut self, seq: Seq, info: PacketRateInfo) {
        if let Some(pkt) = self.map.get_mut(&seq) {
            pkt.rate_info = Some(info);
        }
    }

    /// Pins the route of a sent packet; a retransmission goes there (`None` — the current remote address).
    pub(super) fn set_route(&mut self, seq: Seq, addr: Option<SocketAddr>) {
        if let Some(pkt) = self.map.get_mut(&seq) {
            pkt.remote_addr = addr;
        }
    }

    /// Pins `addr` as the route of every sent packet without one (a route change keeps the flight on the old path).
    pub(super) fn stamp_route(&mut self, addr: SocketAddr) {
        for pkt in self.map.values_mut().filter(|p| p.remote_addr.is_none()) {
            pkt.remote_addr = Some(addr);
        }
    }

    /// Cross-check of the counters against a map walk. `debug_assert_eq!` — the walk is not computed in release.
    pub(super) fn debug_check(&self) {
        debug_assert_eq!(
            (self.inflight(), self.sacked, self.lost, self.highest_sacked),
            self.recount(),
            "outgoing counters diverged from the packet flags"
        );
        debug_assert_eq!(
            self.unsacked_bytes,
            self.recount_bytes(),
            "unsacked_bytes diverged from the lengths of packets without SACK"
        );
        self.debug_check_order();
        let map = &self.map;
        self.unsacked.debug_check(
            map.iter().filter(|(_, p)| !p.sacked).map(|(&s, _)| s),
            "unsacked",
        );
        self.lost_seqs.debug_check(
            map.iter().filter(|(_, p)| p.lost).map(|(&s, _)| s),
            "lost_seqs",
        );
        debug_assert_eq!(self.unsacked.len(), map.len() - self.sacked as usize);
        debug_assert_eq!(self.lost_seqs.len(), self.lost as usize);
    }

    /// The same four values by walking the map: `inflight` and the highest SACK — as they were computed before the counters.
    fn recount(&self) -> (u32, u32, u32, Option<Seq>) {
        let count =
            |keep: fn(&SentPacket) -> bool| self.map.values().filter(|p| keep(p)).count() as u32;
        let highest = self
            .map
            .iter()
            .filter(|(_, p)| p.sacked)
            .map(|(&seq, _)| seq)
            .max();
        (
            count(|p| !p.sacked && !p.lost),
            count(|p| p.sacked),
            count(|p| p.lost),
            highest,
        )
    }

    /// Sum of lengths of packets without SACK by walking the map.
    fn recount_bytes(&self) -> usize {
        let unsacked = self.map.values().filter(|p| !p.sacked);
        unsacked.map(|p| p.packet.len()).sum()
    }

    fn count(&mut self, seq: Seq, pkt: &SentPacket) {
        self.sacked += u32::from(pkt.sacked);
        self.lost += u32::from(pkt.lost);
        if !pkt.sacked {
            self.unsacked_bytes += pkt.packet.len();
        }
        if pkt.sacked {
            self.highest_sacked = self.highest_sacked.max(Some(seq));
        }
    }

    /// Removed packets do not come back and SACK is not cleared: when `sacked == 0` there is no highest SACK any more,
    /// otherwise it is not removed (the removed numbers are below the remaining ones).
    fn uncount(&mut self, pkt: &SentPacket) {
        self.sacked -= u32::from(pkt.sacked);
        self.lost -= u32::from(pkt.lost);
        if !pkt.sacked {
            self.unsacked_bytes -= pkt.packet.len();
        }
        if self.sacked == 0 {
            self.highest_sacked = None;
        }
    }
}

/// Retransmission of one packet: removes the MTU probe padding, counts the retransmission. Result — a copy of the packet and "was a probe".
fn resend_one(pkt: &mut SentPacket, now: Instant) -> (Vec<u8>, bool) {
    let was_probe = std::mem::take(&mut pkt.is_mtu_probe);
    if was_probe {
        strip_probe_padding(&mut pkt.packet);
    }
    pkt.retransmit_count += 1;
    pkt.time_sent = now;
    (pkt.packet.clone(), was_probe)
}

#[cfg(test)]
#[path = "sent_map_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "sent_map_bytes_tests.rs"]
mod bytes_tests;
