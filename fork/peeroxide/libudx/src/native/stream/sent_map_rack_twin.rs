//! Helpers for the RACK index tests: "twins" — a map with the old walk by number (`old`, index off)
//! and a map with the send-order index (`new`). Each action goes into both maps, results are compared at once.
use super::*;
use std::time::{Duration, Instant};

/// Linear congruential generator: the series is repeatable by seed.
pub(super) struct Lcg(pub(super) u64);

impl Lcg {
    /// A number from `0..n`, `n > 0`.
    pub(super) fn below(&mut self, n: u32) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 33) as u32) % n
    }
}

/// A first-transmission DATA of 100 zero bytes, sent at moment `at`, no flags.
pub(super) fn pkt(at: Instant) -> SentPacket {
    SentPacket {
        packet: vec![0u8; 100],
        time_sent: at,
        retransmit_count: 0,
        sacked: false,
        is_mtu_probe: false,
        lost: false,
        rate_info: None,
        remote_addr: None,
    }
}

/// Two maps with one history. Time is virtual: `now` is moved only by `tick` and `send`.
pub(super) struct Twin {
    pub(super) old: SentMap,
    pub(super) new: SentMap,
    pub(super) now: Instant,
    /// Number of the next send.
    pub(super) next: u32,
    /// The smallest number not yet removed by a cumulative ACK.
    pub(super) acked: u32,
    /// How many packets RACK marked over the history (by `new`): the test is non-degenerate if above zero.
    pub(super) rack_losses: usize,
    /// How many SACK blocks marked at least one new number (`sack_block`).
    pub(super) blocks_with_new: usize,
}

impl Twin {
    pub(super) fn starting_at(first: u32) -> Self {
        Self {
            old: SentMap::new(false),
            new: SentMap::new(true),
            now: Instant::now(),
            next: first,
            acked: first,
            rack_losses: 0,
            blocks_with_new: 0,
        }
    }

    pub(super) fn tick(&mut self, ms: u64) {
        self.now += Duration::from_millis(ms);
    }

    /// `count` packets, times strictly ascending (step 1 us).
    pub(super) fn send(&mut self, count: u32) {
        for _ in 0..count {
            self.put_next();
            self.now += Duration::from_micros(1);
        }
    }

    /// `count` packets with one send time (equal times).
    pub(super) fn send_burst(&mut self, count: u32) {
        (0..count).for_each(|_| self.put_next());
    }

    pub(super) fn sack(&mut self, seq: u32) {
        let seen = |p: Option<&SentPacket>| p.map(|p| (p.time_sent, p.retransmit_count));
        let old = seen(self.old.mark_sacked(Seq(seq), self.now));
        let new = seen(self.new.mark_sacked(Seq(seq), self.now));
        assert_eq!(old, new, "SACK no. {seq}: maps diverged");
    }

    /// SACK block `[start, end)`: `old` — `mark_sacked` for each number of the block, `new` — via `unsacked_in`.
    /// Result — numbers marked for the first time; the lists of new ones, `rack().rtt` and `rack().sent` are compared.
    pub(super) fn sack_block(&mut self, start: u32, end: u32) -> Vec<Seq> {
        let len = end.wrapping_sub(start);
        let mut old = Vec::new();
        for i in 0..len {
            let seq = Seq(start.wrapping_add(i));
            if self.old.mark_sacked(seq, self.now).is_some() {
                old.push(seq);
            }
        }
        let mut new = Vec::new();
        for seq in self.new.unsacked_in(Seq(start), Seq(end)) {
            if self.new.mark_sacked(seq, self.now).is_some() {
                new.push(seq);
            }
        }
        assert_eq!(old, new, "SACK block {start}..{end}: new numbers diverged");
        assert_eq!(
            (self.old.rack().rtt, self.old.rack().sent),
            (self.new.rack().rtt, self.new.rack().sent),
            "SACK block {start}..{end}: RACK clocks diverged"
        );
        self.blocks_with_new += usize::from(!new.is_empty());
        new
    }

    /// Cumulative ACK: removes numbers before `ack`; the ones removed in both maps are the same.
    pub(super) fn ack(&mut self, ack: u32) {
        let seen = |m: BTreeMap<Seq, SentPacket>| -> Vec<(Seq, bool, bool)> {
            m.into_iter().map(|(s, p)| (s, p.sacked, p.lost)).collect()
        };
        let old = seen(self.old.remove_before(Seq(ack), self.now));
        let new = seen(self.new.remove_before(Seq(ack), self.now));
        assert_eq!(old, new, "ACK {ack}: different packets removed");
        self.acked = ack;
    }

    pub(super) fn rack(&mut self, reo_wnd: Duration) -> (Vec<Seq>, Option<Duration>) {
        let old = self.old.mark_rack_lost(reo_wnd, self.now);
        let new = self.new.mark_rack_lost(reo_wnd, self.now);
        assert_eq!(old, new, "RACK: index and old walk diverged");
        self.rack_losses += new.0.len();
        new
    }

    pub(super) fn rto(&mut self, reo_wnd: Duration) -> Vec<Seq> {
        let old = self.old.mark_rto_lost(reo_wnd, self.now);
        let new = self.new.mark_rto_lost(reo_wnd, self.now);
        assert_eq!(old, new, "RTO: maps diverged");
        new
    }

    /// Retransmission of all waiting ones in ascending number order, retransmission time is `now`. Result — how many were retransmitted.
    pub(super) fn resend_all(&mut self) -> u32 {
        let mut count = 0;
        while let Some(seq) = self.old.first_lost(None) {
            assert_eq!(self.new.first_lost(None), Some(seq), "first lost");
            let old = self.old.resend(seq, self.now);
            let new = self.new.resend(seq, self.now);
            assert_eq!(old, new, "retransmission no. {}", seq.0);
            count += 1;
        }
        assert_eq!(self.new.first_lost(None), None, "awaiting retransmission");
        count
    }

    /// The states are equal: entries, counters, index keys against a recount over the map, `debug_check` of both.
    /// `rack_visited` is not compared: the paths count differently.
    pub(super) fn check(&self) {
        let entries = |m: &SentMap| -> Vec<_> {
            m.iter()
                .map(|(&s, p)| (s, p.sacked, p.lost, p.retransmit_count, p.time_sent))
                .collect()
        };
        assert_eq!(entries(&self.old), entries(&self.new), "map entries");
        let counters = |m: &SentMap| {
            let (inflight, sacked, lost) = (m.inflight(), m.sacked(), m.lost());
            (inflight, sacked, lost, m.unsacked_bytes(), m.resent())
        };
        assert_eq!(counters(&self.old), counters(&self.new), "map counters");
        assert_eq!(
            self.new.order.keys(),
            Some(self.recount_keys()),
            "index != recount over the map"
        );
        assert_eq!(self.old.order.keys(), None, "disabled index created keys");
        self.check_first_lost();
        self.old.debug_check();
        self.new.debug_check();
    }

    /// `first_lost` of `new` equals a walk of the `new` map: from the head and from the middle of the map.
    fn check_first_lost(&self) {
        let walk = |after: Option<Seq>| {
            let mut lost = self.new.iter().filter(|(_, p)| p.lost).map(|(&s, _)| s);
            lost.find(|s| after.is_none_or(|a| *s > a))
        };
        let middle = self.new.keys().nth(self.new.len() / 2).copied();
        for after in [None, middle] {
            assert_eq!(
                self.new.first_lost(after),
                walk(after),
                "first_lost({after:?})"
            );
        }
    }

    /// RACK-eligible packets in `new`: without SACK and loss, sent before `rack.sent` (0 for `None`).
    pub(super) fn eligible(&self) -> u32 {
        let Some(rack_sent) = self.new.rack().sent else {
            return 0;
        };
        let fit = |p: &&SentPacket| !p.sacked && !p.lost && p.time_sent < rack_sent;
        self.new.values().filter(fit).count() as u32
    }

    fn put_next(&mut self) {
        let seq = Seq(self.next);
        self.old.insert(seq, pkt(self.now));
        self.new.insert(seq, pkt(self.now));
        self.next = self.next.wrapping_add(1);
    }

    /// Index keys "from scratch": `new` entries without SACK and loss, in ascending `(time, raw number)`.
    fn recount_keys(&self) -> Vec<(Instant, u32)> {
        let flying = self.new.iter().filter(|(_, p)| !p.sacked && !p.lost);
        let mut keys: Vec<_> = flying.map(|(s, p)| (p.time_sent, s.0)).collect();
        keys.sort();
        keys
    }
}
