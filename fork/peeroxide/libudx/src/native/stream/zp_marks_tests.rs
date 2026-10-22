//! Mark counters of the `ZP` instrument: grow on a synthetic loss along the path of each source.
use super::test_support::{stream_inner, with_sent};
use super::*;

fn sent(count: u32) -> StreamInner {
    let mut inner = stream_inner();
    with_sent(&mut inner, 0, count);
    inner
}

#[test]
fn reo_fire_counts_timer_marks_and_enter() {
    // given: 10 packets, SACK [5, 7): 0..4 lost by RACK
    let mut inner = sent(10);
    inner.on_sack(&[SackRange { start: 5, end: 7 }]);
    // when: the mark is made by the reo timer
    inner.on_reo_fire();
    // then: marks in marks_timer, recovery entry in marks_enter, the rest zero
    let z = &inner.zp;
    assert_eq!(
        (z.marks_ack, z.marks_timer, z.marks_enter, z.marks_rto),
        (0, 5, 5, 0)
    );
}

#[test]
fn rto_counts_rto_marks() {
    let mut inner = sent(4);
    inner.mark_rto_losses(Instant::now() + std::time::Duration::from_secs(3600));
    let z = &inner.zp;
    assert_eq!((z.marks_ack, z.marks_timer, z.marks_rto), (0, 0, 4));
}

#[test]
fn zp_view_reflects_sender_state() {
    let mut inner = sent(6);
    inner.on_sack(&[SackRange { start: 4, end: 6 }]);
    let v = inner.zp_view();
    assert_eq!((v.remote_acked, v.seq, v.map_len, v.map_min), (0, 6, 6, 0));
    assert_eq!((v.sacked, v.lost, v.inflight, v.cwnd_cmp), (2, 0, 4, 4));
    assert_eq!(v.cwnd_lim, inner.congestion.send_limit(inner.send_rwnd));
}
