//! `SentMap` sequence number index tests: `unsacked_in`, `first_lost` against a map walk, the `debug_check` cross-check.
use super::*;
use std::time::Duration;

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn pkt(at: Instant) -> SentPacket {
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

/// A map of `count` packets from `first`, packet `i` sent at `base + i ms`.
fn filled(first: u32, count: u32, base: Instant) -> SentMap {
    let mut map = SentMap::default();
    for i in 0..count {
        map.insert(Seq(first.wrapping_add(i)), pkt(base + ms(u64::from(i))));
    }
    map
}

/// `first_lost` by walking the map.
fn lost_by_walk(map: &SentMap, after: Option<Seq>) -> Option<Seq> {
    let mut lost = map.iter().filter(|(_, p)| p.lost).map(|(&s, _)| s);
    lost.find(|s| after.is_none_or(|a| *s > a))
}

fn assert_first_lost_matches_walk(map: &SentMap) {
    let middle = map.keys().nth(map.len() / 2).copied();
    for after in [None, middle] {
        assert_eq!(
            map.first_lost(after),
            lost_by_walk(map, after),
            "after = {after:?}"
        );
    }
    map.debug_check();
}

mod unsacked_in {
    use super::*;

    #[test]
    fn block_over_marked_numbers_gives_only_the_new() {
        // given: 2 000 packets, SACK on 0..1990
        let base = Instant::now();
        let mut map = filled(0, 2000, base);
        (0..1990).for_each(|s| {
            let _ = map.mark_sacked(Seq(s), base);
        });
        // when / then: block 0..2000 — only 10 new
        let fresh = map.unsacked_in(Seq(0), Seq(2000));
        assert_eq!(fresh, (1990..2000).map(Seq).collect::<Vec<_>>());
    }

    #[test]
    fn fully_marked_block_is_empty() {
        let base = Instant::now();
        let mut map = filled(0, 20, base);
        (5..15).for_each(|s| {
            let _ = map.mark_sacked(Seq(s), base);
        });
        assert!(map.unsacked_in(Seq(5), Seq(15)).is_empty());
    }

    #[test]
    fn block_across_2_pow_32_gives_both_sides() {
        let map = filled(u32::MAX - 5, 10, Instant::now());
        let fresh = map.unsacked_in(Seq(u32::MAX - 3), Seq(3));
        let want = [u32::MAX - 3, u32::MAX - 2, u32::MAX - 1, u32::MAX, 0, 1, 2];
        assert_eq!(fresh, want.map(Seq).to_vec());
    }

    #[test]
    fn empty_or_reversed_block_is_empty() {
        let map = filled(0, 10, Instant::now());
        assert!(map.unsacked_in(Seq(4), Seq(4)).is_empty());
        assert!(map.unsacked_in(Seq(6), Seq(4)).is_empty());
    }

    #[test]
    fn lost_but_not_sacked_is_included() {
        let base = Instant::now();
        let mut map = filled(0, 5, base);
        map.mark_rto_lost(Duration::ZERO, base + ms(1000));
        assert!(map.lost() > 0);
        assert_eq!(map.unsacked_in(Seq(0), Seq(5)).len(), 5);
    }
}

mod lost_seqs {
    use super::*;

    #[test]
    fn follows_rack_rto_sack_resend_and_cumulative_ack() {
        let base = Instant::now();
        let mut map = filled(0, 10, base);
        map.mark_sacked(Seq(9), base + ms(20));
        // RACK: everything up to 9 s expired
        let (lost, _) = map.mark_rack_lost(Duration::ZERO, base + ms(5000));
        assert_eq!(lost.len(), 9);
        assert_first_lost_matches_walk(&map);
        // SACK of a lost one removes it
        map.mark_sacked(Seq(2), base + ms(5001));
        assert_first_lost_matches_walk(&map);
        // retransmission
        map.resend(Seq(0), base + ms(5002));
        assert_eq!(map.first_lost(None), Some(Seq(1)));
        assert_first_lost_matches_walk(&map);
        // cumulative ACK
        map.remove_before(Seq(4), base + ms(5003));
        assert_eq!(map.first_lost(None), Some(Seq(4)));
        assert_first_lost_matches_walk(&map);
        // RTO: marks the not yet marked (Seq(0) after retransmission, Seq(9) with SACK is untouched)
        map.mark_rto_lost(Duration::ZERO, base + ms(9000));
        assert_first_lost_matches_walk(&map);
    }

    #[test]
    fn lost_far_behind_thousands_of_unlost_is_found() {
        let base = Instant::now();
        let mut map = filled(0, 3100, base);
        // all marked by RTO, all retransmitted except 3 050: one lost, behind 3 000 not lost
        map.mark_rto_lost(Duration::ZERO, base + ms(100_000));
        (0..3100).filter(|&s| s != 3050).for_each(|s| {
            let _ = map.resend(Seq(s), base + ms(100_001));
        });
        assert_eq!(map.lost(), 1);
        assert_eq!(map.first_lost(None), Some(Seq(3050)));
        assert_first_lost_matches_walk(&map);
        assert!(map.first_lost(Some(Seq(3000))).is_some());
    }
}

#[cfg(debug_assertions)]
mod seq_index_check {
    use super::*;

    #[test]
    #[should_panic(expected = "diverged from the map")]
    fn debug_check_catches_a_wrong_expectation() {
        let mut index = SeqIndex::default();
        index.add(Seq(1));
        index.debug_check([Seq(1), Seq(2)].into_iter(), "test");
    }
}
