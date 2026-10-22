//! The fields of the RTO-failure WARN (`rto_failure_fields`) on a bare `StreamInner`.

use super::*;
use crate::native::stream::test_support::stream_inner;

#[test]
fn last_rx_none_until_packet() {
    let mut inner = stream_inner();
    assert!(inner.last_rx.is_none());
    inner.on_ack_packet(0, 1 << 20, &[]);
    assert!(inner.last_rx.is_some());
}

#[test]
fn fields_without_rx() {
    let inner = stream_inner();
    let f = rto_failure_fields(&inner, Instant::now(), Duration::from_millis(5));
    assert_eq!(f.last_rx_ms, -1);
    assert_eq!(f.rx, 0);
    assert_eq!(f.age_ms, 5);
    assert_eq!(f.rto_ms, u64::from(inner.rto));
}

#[test]
fn fields_after_ack() {
    let mut inner = stream_inner();
    crate::native::stream::test_support::with_sent(&mut inner, 0, 5);
    inner.on_ack_packet(3, 1 << 20, &[]);
    let f = rto_failure_fields(&inner, Instant::now(), Duration::ZERO);
    assert_eq!(f.rx, 1);
    assert_eq!(f.remote_acked, 3);
    assert!(
        (0..=50).contains(&f.last_rx_ms),
        "last_rx_ms {}",
        f.last_rx_ms
    );
    assert_eq!(f.unacked, 2);
}

#[test]
fn end_seq_reported() {
    let mut inner = stream_inner();
    assert_eq!(
        rto_failure_fields(&inner, Instant::now(), Duration::ZERO).end_seq,
        None
    );
    let n = inner.reserve_end_seq();
    assert_eq!(
        rto_failure_fields(&inner, Instant::now(), Duration::ZERO).end_seq,
        Some(n)
    );
}
