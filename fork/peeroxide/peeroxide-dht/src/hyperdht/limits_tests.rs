use super::*;

#[test]
fn default_limits_are_all_none() {
    assert_eq!(StreamLimits::default().recv_window_max, None);
    assert_eq!(StreamLimits::default().send_window_max, None);
    assert_eq!(StreamLimits::default().write_high_watermark, None);
}

#[tokio::test]
async fn apply_sets_only_the_given_knobs() {
    let runtime = libudx::UdxRuntime::new().expect("runtime");
    let stream = runtime.create_stream(1).await.expect("stream");
    let raw = stream.into_async_stream();
    let before = raw.recv_window_max();

    StreamLimits::default().apply(&raw);
    assert_eq!(raw.recv_window_max(), before);

    StreamLimits {
        recv_window_max: Some(131_072),
        ..StreamLimits::default()
    }
    .apply(&raw);
    assert_eq!(raw.recv_window_max(), 131_072);
}
