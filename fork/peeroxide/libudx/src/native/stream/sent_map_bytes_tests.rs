//! Tests of `SentMap::unsacked_bytes`: the sum of `packet.len()` over entries without SACK equals a map walk after every operation.
use super::*;
use std::time::Duration;

/// A packet of length `len`, sent at moment `at`.
fn packet(len: usize, at: Instant) -> SentPacket {
    SentPacket {
        packet: vec![0u8; len],
        time_sent: at,
        retransmit_count: 0,
        sacked: false,
        is_mtu_probe: false,
        lost: false,
        rate_info: None,
        remote_addr: None,
    }
}

/// A map of packets numbered 0, 1, ... with given lengths; times grow by a millisecond.
fn map_of(lens: &[usize]) -> SentMap {
    let base = Instant::now();
    let mut map = SentMap::default();
    for (i, &len) in lens.iter().enumerate() {
        let at = base + std::time::Duration::from_millis(i as u64);
        map.insert(Seq(i as u32), packet(len, at));
    }
    map
}

#[test]
fn insert_adds_packet_length() {
    // given: a map of packets of 100 and 250 bytes
    let mut map = map_of(&[100, 250]);
    assert_eq!(map.unsacked_bytes(), 350);
    // when: we add a third
    map.insert(Seq(2), packet(40, Instant::now()));
    // then
    assert_eq!(map.unsacked_bytes(), 390);
    map.debug_check();
}

#[test]
fn first_sack_subtracts_and_repeat_sack_does_not() {
    // given: three packets 100, 250, 40
    let mut map = map_of(&[100, 250, 40]);
    // when: SACK of packet 1, then a repeat
    assert!(map.mark_sacked(Seq(1), Instant::now()).is_some());
    // then: 250 subtracted once
    assert_eq!(map.unsacked_bytes(), 140);
    assert!(map.mark_sacked(Seq(1), Instant::now()).is_none());
    assert_eq!(map.unsacked_bytes(), 140);
    map.debug_check();
}

#[test]
fn remove_before_subtracts_only_unsacked() {
    // given: packets 100, 250, 40, 60; SACK on the second
    let mut map = map_of(&[100, 250, 40, 60]);
    map.mark_sacked(Seq(1), Instant::now());
    assert_eq!(map.unsacked_bytes(), 200);
    // when: a cumulative ACK removes 0 and 1
    map.remove_before(Seq(2), Instant::now());
    // then: only 100 subtracted (250 was already subtracted at SACK)
    assert_eq!(map.unsacked_bytes(), 100);
    map.debug_check();
}

#[test]
fn lost_mark_and_resend_lost_keep_bytes_of_plain_packets() {
    // given: packets 100, 250, 40; SACK on the last, 0 and 1 sent before it
    let mut map = map_of(&[100, 250, 40]);
    map.mark_sacked(Seq(2), Instant::now());
    assert_eq!(map.unsacked_bytes(), 350);
    // when: a loss is marked
    let lost = map
        .mark_rack_lost(Duration::ZERO, Instant::now() + Duration::from_secs(3600))
        .0;
    // then: a loss does not touch bytes
    assert_eq!(lost.len(), 2);
    assert_eq!(map.unsacked_bytes(), 350);
    map.debug_check();
    // when: fast retransmission of regular packets
    let copies: Vec<_> = std::iter::from_fn(|| {
        map.first_lost(None)
            .and_then(|s| map.resend(s, Instant::now()))
    })
    .collect();
    // then: lengths unchanged
    assert_eq!(copies.len(), 2);
    assert!(copies.iter().all(|(_, probe)| !probe));
    assert_eq!(map.unsacked_bytes(), 350);
    map.debug_check();
}
