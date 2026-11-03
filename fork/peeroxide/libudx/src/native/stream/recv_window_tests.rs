//! Window accounting: saturation, release, the update threshold and the floor of the maximum.
use super::*;
use crate::native::stream::test_support::stream_inner;

fn with_unread(unread: usize) -> StreamInner {
    let mut inner = stream_inner();
    inner.note_unread(unread);
    inner
}

#[test]
fn window_saturates_at_zero() {
    let inner = with_unread(DEFAULT_RWND as usize + 5_000);
    assert_eq!(inner.advertised_window(), 0);
}

#[test]
fn window_is_the_maximum_less_unread_and_buffered() {
    let mut inner = with_unread(1000);
    inner.recv_buf_bytes = 500;
    assert_eq!(inner.advertised_window(), DEFAULT_RWND - 1500);
    assert_eq!(inner.recv_queued(), 1500);
}

#[test]
fn release_saturates_at_zero() {
    let mut inner = with_unread(100);
    inner.release_read(1_000_000);
    assert_eq!(inner.unread, 0);
    assert_eq!(inner.advertised_window(), DEFAULT_RWND);
}

#[test]
fn release_reports_an_update_only_when_a_step_is_crossed() {
    let mut inner = with_unread(DEFAULT_RWND as usize);
    assert!(inner.release_read(1_048_577), "0 -> past the first step");
    assert!(!inner.release_read(1000), "inside the same step");
}

#[test]
fn update_due_crossing_a_multiple_of_the_step() {
    let max = 4_000;
    assert!(window_update_due(0, 1_000, max));
    assert!(window_update_due(900, 2_100, max));
    assert!(!window_update_due(1_000, 1_999, max));
    assert!(!window_update_due(1_500, 1_000, max));
}

#[test]
fn update_not_due_for_a_reader_that_keeps_up() {
    assert!(!window_update_due(3_100, 4_000, 4_000));
}

#[test]
fn maximum_has_a_floor() {
    let mut inner = stream_inner();
    inner.set_recv_window_max(1000);
    assert_eq!(inner.recv_rwnd_max, MIN_RECV_WINDOW);
    assert_eq!(inner.advertised_window(), MIN_RECV_WINDOW);
}

#[test]
fn growing_the_maximum_asks_for_an_update() {
    let mut inner = stream_inner();
    inner.set_recv_window_max(MIN_RECV_WINDOW);
    inner.note_unread(MIN_RECV_WINDOW as usize);
    assert!(inner.set_recv_window_max(1 << 20), "closed window reopened");
    assert!(!inner.set_recv_window_max(1 << 10), "shrinking is silent");
}

#[test]
fn out_of_order_fits_only_below_the_maximum() {
    let mut inner = stream_inner();
    inner.set_recv_window_max(MIN_RECV_WINDOW);
    inner.note_unread(MIN_RECV_WINDOW as usize - 10);
    assert!(inner.ooo_fits(10));
    assert!(!inner.ooo_fits(11));
}

#[test]
fn stamp_window_skips_destroy() {
    let mut data = vec![0u8; 20];
    data[2] = FLAG_DATA;
    stamp_window(&mut data, 0x0102_0304);
    assert_eq!(&data[8..12], &0x0102_0304u32.to_le_bytes());
    let mut destroy = vec![0u8; 20];
    destroy[2] = FLAG_DESTROY;
    stamp_window(&mut destroy, 7);
    assert_eq!(&destroy[8..12], &[0, 0, 0, 0]);
}
