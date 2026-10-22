//! Tests of the RACK send-order index: the index follows the flight, a walk by the index gives the same as the old walk
//! by numbers, the mode is taken from `UDX_FIX` when the stream is created. Twins — `sent_map_rack_twin.rs`.
use super::twin::{Twin, pkt};
use super::*;
use crate::native::header::SackRange;
use crate::native::stream::test_support::{stream_inner, with_sent};
use std::time::{Duration, Instant};

mod send_order_follows_the_flight {
    use super::*;

    #[test]
    fn sent_packets_enter_by_time() {
        // given: 5 sends 1 ms apart
        let base = Instant::now();
        let map = indexed(0, 5, base);
        // then: keys in ascending time order, as many as in flight
        let expected: Vec<(Instant, u32)> = (0..5).map(|i| (base + ms(u64::from(i)), i)).collect();
        assert_eq!(map.order.keys(), Some(expected));
        assert_eq!(map.order.len(), 5);
        assert_eq!(map.inflight(), 5);
    }

    #[test]
    fn sack_removes_the_packet() {
        // given: 5 sends
        let base = Instant::now();
        let mut map = indexed(0, 5, base);
        // when: SACK no. 2
        let first = map.mark_sacked(Seq(2), base + ms(10));
        // then: the packet is accepted for the first time, exactly it left the index
        assert!(first.is_some(), "SACK no. 2 accepted for the first time");
        assert_eq!(indexed_seqs(&map), vec![0, 1, 3, 4]);
        assert_eq!(map.order.len(), 4);
        map.debug_check();
    }

    #[test]
    fn sack_of_a_lost_packet_leaves_the_index_alone() {
        // given: 5 sends 1 ms apart; RTO at 2.5 ms marked no. 0-2 (the head and all whose time ran out); no. 3, 4 stay in the index
        let base = Instant::now();
        let mut map = indexed(0, 5, base);
        map.mark_rto_lost(Duration::ZERO, base + Duration::from_micros(2500));
        assert_eq!(indexed_seqs(&map), vec![3, 4]);
        let before = map.order.keys();
        // when: SACK of the lost no. 1
        map.mark_sacked(Seq(1), base + ms(10));
        // then: same index (the lost one was not in it), counters agree with the map
        assert_eq!(map.order.keys(), before);
        assert_eq!((map.lost(), map.sacked()), (2, 1));
        map.debug_check();
    }

    #[test]
    fn cumulative_ack_across_the_wrap_keeps_only_the_tail() {
        // given: 6 sends starting at MAX-2: MAX-2, MAX-1, MAX, 0, 1, 2
        let base = Instant::now();
        let mut map = indexed(u32::MAX - 2, 6, base);
        // when: ACK 2 removes five
        let removed = map.remove_before(Seq(2), base + ms(10));
        // then: only no. 2 with its time is in the index
        assert_eq!(removed.len(), 5);
        assert_eq!(map.order.keys(), Some(vec![(base + ms(5), 2)]));
        map.debug_check();
    }

    #[test]
    fn rto_removes_and_resend_returns_the_packet_to_the_tail() {
        // given: 3 sends, an RTO far in the future marked all — the index is empty; then no. 3, no. 4 more
        let base = Instant::now();
        let mut map = indexed(0, 3, base);
        map.mark_rto_lost(Duration::ZERO, base + Duration::from_secs(3600));
        assert_eq!(map.order.len(), 0, "lost ones are outside the index");
        map.insert(Seq(3), pkt(base + ms(10)));
        map.insert(Seq(4), pkt(base + ms(11)));
        // when: retransmission of no. 0
        let at = base + ms(20);
        assert!(map.resend(Seq(0), at).is_some());
        // then: no. 0 is last, with the new time
        let expected = vec![(base + ms(10), 3), (base + ms(11), 4), (at, 0)];
        assert_eq!(map.order.keys(), Some(expected));
        map.debug_check();
    }

    #[test]
    fn rack_loss_removes_the_lost_packets() {
        // given: 4 sends 1 ms apart, SACK no. 3 after 10 ms
        let base = Instant::now();
        let mut map = indexed(0, 4, base);
        map.mark_sacked(Seq(3), base + ms(10));
        // when: the deadline passed (rtt 7 ms, no. 2 sent at the 2nd ms)
        let (lost, _) = map.mark_rack_lost(Duration::ZERO, base + ms(10));
        // then: no. 0-2 are lost and left the index
        assert_eq!(lost, vec![Seq(0), Seq(1), Seq(2)]);
        assert_eq!(map.order.len(), 0);
        assert_eq!(map.lost(), 3);
        map.debug_check();
    }

    #[test]
    fn default_map_keeps_the_index() {
        // given / when: the default map went through all kinds of events
        let map = after_every_kind_of_event(SentMap::default());
        // then
        assert!(map.order.keys().is_some());
        assert!(map.order.is_enabled());
        map.debug_check();
    }

    #[test]
    fn map_created_disabled_keeps_no_index() {
        // given / when: a `new(false)` map went through all kinds of events
        let map = after_every_kind_of_event(SentMap::new(false));
        // then
        assert_eq!(map.order.keys(), None);
        assert_eq!(map.order.len(), 0);
        map.debug_check();
    }
}

mod indexed_walk_matches_the_scan {
    use super::*;

    #[test]
    fn without_rack_sent_nothing_is_walked() {
        // given: 3 sends without deliveries, the previous walk count is 7
        let mut twin = Twin::starting_at(0);
        twin.send(3);
        twin.old.rack_visited = 7;
        twin.new.rack_visited = 7;
        // when
        let result = twin.rack(Duration::ZERO);
        // then: no losses, no timer, the count untouched on every path
        assert_eq!(result, (vec![], None));
        assert_eq!((twin.old.rack_visited, twin.new.rack_visited), (7, 7));
    }

    #[test]
    fn hole_under_thousands_of_sacks_costs_three_steps() {
        // given: 3000 sends, SACK no. 3..=2999, a second passed
        let mut twin = Twin::starting_at(0);
        twin.send(3000);
        sack_range(&mut twin, 3..3000);
        twin.tick(1000);
        // when
        let (lost, _) = twin.rack(Duration::ZERO);
        // then: the hole no. 0-2 is lost; the index passed 3 entries, the old walk all 3000 (with the stopper entry no. 2999)
        assert_eq!(lost, seqs(&[0, 1, 2]));
        assert_eq!(twin.new.rack_visited, 3);
        assert_eq!(twin.old.rack_visited, 3000);
        twin.check();
    }

    #[test]
    fn disabled_map_keeps_the_old_visit_count() {
        // given: 13 sends, SACK no. 3..=12, a second passed
        let mut twin = Twin::starting_at(0);
        twin.send(13);
        sack_range(&mut twin, 3..13);
        twin.tick(1000);
        // when
        let (lost, _) = twin.rack(Duration::ZERO);
        // then: same losses; the disabled one counts with the stopper (13), the enabled one only the eligible (3)
        assert_eq!(lost, seqs(&[0, 1, 2]));
        assert_eq!(twin.old.rack_visited, 13);
        assert_eq!(twin.new.rack_visited, 3);
    }

    #[test]
    fn young_hole_only_arms_the_timer() {
        // given: 10 sends, SACK no. 9 at once; without allowance `reo = 0` no. 0 is already overdue (t0 + rtt <= now)
        let mut twin = Twin::starting_at(0);
        twin.send(10);
        twin.sack(9);
        // when: allowance 5 ms
        let (lost, timeout) = twin.rack(Duration::from_millis(5));
        // then: no losses, the remainder goes into the timer (equality of remainders checked by `Twin::rack`)
        assert!(lost.is_empty(), "the hole is younger than the deadline");
        assert!(timeout.is_some(), "the remainder did not reach the timer");
    }

    #[test]
    fn retransmit_sent_after_rack_sent_is_not_walked() {
        // given: 5 sends, SACK no. 4, an RTO after 500 ms marked no. 0-3, all retransmitted
        let mut twin = Twin::starting_at(0);
        twin.send(5);
        twin.tick(1);
        twin.sack(4);
        twin.tick(500);
        assert_eq!(twin.rto(Duration::ZERO), seqs(&[0, 1, 2, 3]));
        assert_eq!(twin.resend_all(), 4);
        // when
        let result = twin.rack(Duration::ZERO);
        // then: retransmissions later than `rack.sent` are ineligible; the old walk passed 5 entries, the index none
        assert_eq!(result, (vec![], None));
        assert_eq!(twin.old.rack_visited, 5);
        assert_eq!(twin.new.rack_visited, 0);
        assert_eq!(twin.new.rack_visited, twin.eligible());
    }

    #[test]
    fn equal_send_times_are_not_older_than_rack_sent() {
        // given: 4 sends at one moment, SACK no. 3 after 1 ms
        let mut twin = Twin::starting_at(0);
        twin.send_burst(4);
        twin.tick(1);
        twin.sack(3);
        // when
        let result = twin.rack(Duration::ZERO);
        // then: strict "earlier" — no. 0-2 are ineligible; the old walk stopped at no. 0, the index entered nowhere
        assert_eq!(result, (vec![], None));
        assert_eq!(twin.old.rack_visited, 1);
        assert_eq!(twin.new.rack_visited, 0);
    }

    #[test]
    fn losses_across_the_wrap_come_in_seq_order() {
        // given: 8 sends starting at MAX-3 (..., MAX, 0, 1, 2, 3), SACK no. 3, a second passed
        let mut twin = Twin::starting_at(u32::MAX - 3);
        twin.send(8);
        twin.sack(3);
        twin.tick(1000);
        // when
        let (lost, _) = twin.rack(Duration::ZERO);
        // then: seven losses by `Seq`, MAX before 0
        let expected = seqs(&[u32::MAX - 3, u32::MAX - 2, u32::MAX - 1, u32::MAX, 0, 1, 2]);
        assert_eq!(lost, expected);
        twin.check();
    }

    #[test]
    fn equal_times_across_the_wrap_still_sort_by_seq() {
        // given: 4 sends at one moment from MAX-1 (MAX-1, MAX, 0, 1), then no. 2 1 ms later and SACK no. 2
        let mut twin = Twin::starting_at(u32::MAX - 1);
        twin.send_burst(4);
        twin.tick(1);
        twin.send(1);
        twin.sack(2);
        twin.tick(1000);
        // when: in the index equal times go by raw number (0, 1, MAX-1, MAX)
        let (lost, _) = twin.rack(Duration::ZERO);
        // then: the result is still by `Seq`
        assert_eq!(lost, seqs(&[u32::MAX - 1, u32::MAX, 0, 1]));
        twin.check();
    }
}

mod stream_keeps_the_index {
    use super::*;

    #[test]
    fn stream_inner_always_keeps_the_index() {
        // given: the default stream (env does not matter: the `rack` token changes nothing)
        let inner = stream_inner();
        // then
        assert!(inner.outgoing.order.is_enabled());
    }

    #[test]
    fn stream_path_loses_the_same_packets_with_and_without_the_index() {
        // given: two streams, 20 packets from MAX-5 (wrap), SACK on the last; one with the index, the other without
        let mut plain = stream_with_tail_sacked(false);
        let mut with_index = stream_with_tail_sacked(true);
        // when: RACK losses with a single `now` an hour later
        let now = Instant::now() + Duration::from_secs(3600);
        let from_plain = plain.detect_losses_at(now);
        let from_index = with_index.detect_losses_at(now);
        // then: one number and timer, the same numbers, counters agree in both
        assert_eq!(from_index, from_plain);
        assert_eq!(from_index.0, 19, "all except the SACKed one");
        assert_eq!(lost_seqs(&with_index), lost_seqs(&plain));
        plain.debug_check_counters();
        with_index.debug_check_counters();
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn seqs(numbers: &[u32]) -> Vec<Seq> {
    numbers.iter().map(|&n| Seq(n)).collect()
}

/// A map with the index: `count` first-transmission packets from number `first`, packet `i` sent at `base + i ms`.
fn indexed(first: u32, count: u32, base: Instant) -> SentMap {
    let mut map = SentMap::new(true);
    for i in 0..count {
        map.insert(Seq(first.wrapping_add(i)), pkt(base + ms(u64::from(i))));
    }
    map
}

/// Raw index key numbers in ascending key order.
fn indexed_seqs(map: &SentMap) -> Vec<u32> {
    let keys = map.order.keys().expect("index enabled");
    keys.into_iter().map(|(_, seq)| seq).collect()
}

/// All packet event kinds: send, SACK, RACK loss, retransmission, cumulative ACK.
fn after_every_kind_of_event(mut map: SentMap) -> SentMap {
    let base = Instant::now();
    (0..4).for_each(|i| map.insert(Seq(i), pkt(base + ms(u64::from(i)))));
    map.mark_sacked(Seq(3), base + ms(10));
    map.mark_rack_lost(Duration::ZERO, base + ms(10));
    map.resend(Seq(0), base + ms(20));
    map.remove_before(Seq(1), base + ms(21));
    map
}

fn sack_range(twin: &mut Twin, seqs: std::ops::Range<u32>) {
    seqs.for_each(|seq| twin.sack(seq));
}

/// A stream with a `rack_index` map: 20 packets from `MAX - 5` (wrap across 2^32), SACK on the last.
fn stream_with_tail_sacked(rack_index: bool) -> StreamInner {
    let mut inner = stream_inner();
    inner.outgoing = SentMap::new(rack_index);
    with_sent(&mut inner, u32::MAX - 5, 20);
    let last = (u32::MAX - 5).wrapping_add(19);
    let end = last.wrapping_add(1);
    inner.on_sack(&[SackRange { start: last, end }]);
    inner
}

/// Numbers with `lost` in map order.
fn lost_seqs(inner: &StreamInner) -> Vec<u32> {
    let lost = inner.outgoing.iter().filter(|(_, p)| p.lost);
    lost.map(|(seq, _)| seq.0).collect()
}
