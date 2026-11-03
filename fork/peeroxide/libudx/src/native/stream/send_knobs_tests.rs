//! Sender knob tests: the floor and the pass-through of the setters.
use super::super::test_support::stream_inner;
use super::*;

#[test]
fn send_window_max_has_a_floor() {
    let mut inner = stream_inner();
    assert_eq!(inner.send_window_max, u32::MAX);
    inner.set_send_window_max(0);
    assert_eq!(inner.send_window_max, MIN_SEND_WINDOW);
    inner.set_send_window_max(100_000);
    assert_eq!(inner.send_window_max, 100_000);
}

#[test]
fn effective_window_is_the_smaller_of_peer_and_cap() {
    let mut inner = stream_inner();
    inner.send_rwnd = 50_000;
    assert_eq!(inner.effective_send_rwnd(), 50_000);
    inner.set_send_window_max(10_000);
    assert_eq!(inner.effective_send_rwnd(), 10_000);
    inner.send_rwnd = 0;
    assert_eq!(inner.effective_send_rwnd(), 0);
}

#[test]
fn watermark_has_no_floor() {
    let mut inner = stream_inner();
    inner.set_write_high_watermark(0);
    assert_eq!(inner.write_high_watermark, 0);
}
