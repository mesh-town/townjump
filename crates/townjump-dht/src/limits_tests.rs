use super::*;

#[test]
fn stream_limits_default_is_all_none() {
    let limits = StreamLimits::default();

    assert_eq!(limits.recv_window_max, None);
    assert_eq!(limits.send_window_max, None);
    assert_eq!(limits.write_high_watermark, None);
}
