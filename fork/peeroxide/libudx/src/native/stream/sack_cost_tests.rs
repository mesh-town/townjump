//! ACK cost: a SACK block almost entirely marked earlier costs by the count of new numbers and holes, not by block width
//! (C has the `inflight_queue` / `retransmit_queue` lists, udx.c). The timing test is release-only.
use crate::native::header::SackRange;
use crate::native::stream::test_support::{stream_inner, with_sent};
use crate::native::stream::{DEFAULT_RWND, StreamInner};
use std::time::Instant;

fn blocks(last_end: u32) -> [SackRange; 3] {
    [
        SackRange {
            start: 101,
            end: 700,
        },
        SackRange {
            start: 701,
            end: 1300,
        },
        SackRange {
            start: 1301,
            end: last_end,
        },
    ]
}

/// A map of 5,000 entries (100..5099), holes 100/700/1300, SACK at 1,887.
fn primed() -> StreamInner {
    let mut inner = stream_inner();
    with_sent(&mut inner, 0, 5100);
    inner.on_ack_packet(100, DEFAULT_RWND, &blocks(1990));
    inner
}

mod one_hole_under_thousands_of_sacks {
    use super::*;

    #[test]
    fn measured_ack_marks_only_the_new_ten() {
        // given: the map after the first ACK
        let mut inner = primed();
        assert_eq!(inner.outgoing.sacked(), 1887);
        let (inflight, lost) = (inner.outgoing.inflight(), inner.outgoing.lost());
        // when: the ACK under test, coverage 1,897, 10 new
        inner.on_ack_packet(100, DEFAULT_RWND, &blocks(2000));
        // then: SACK at 1,897; in flight drops by 10 plus the new losses; counters agree
        assert_eq!(inner.outgoing.sacked(), 1897);
        let new_lost = inner.outgoing.lost() - lost;
        assert_eq!(inner.outgoing.inflight() + new_lost, inflight - 10);
        inner.debug_check_counters();
    }

    #[test]
    fn measured_ack_walks_at_most_the_three_holes() {
        // given / when: same
        let mut inner = primed();
        inner.on_ack_packet(100, DEFAULT_RWND, &blocks(2000));
        // then: RACK passed at most three holes (by number it would be ~2,000)
        let visited = inner.outgoing.rack_visited;
        assert!(visited <= 3, "rack_visited = {visited}");
    }

    #[test]
    #[cfg_attr(debug_assertions, ignore = "timing is release-only")]
    fn apply_acks_costs_at_most_20_us() {
        // given: 21 fresh maps
        let mut times = Vec::new();
        for _ in 0..21 {
            let mut inner = primed();
            // when: time only around the measured ACK
            let started = Instant::now();
            inner.on_ack_packet(100, DEFAULT_RWND, &blocks(2000));
            times.push(started.elapsed());
        }
        times.sort();
        eprintln!("apply_acks: min {:?}, median {:?}", times[0], times[10]);
        // then
        assert!(times[0].as_micros() <= 20, "min = {:?}", times[0]);
    }
}
