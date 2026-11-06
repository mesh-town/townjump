//! The debug line printed on every RTO firing that has something to retransmit.
use crate::native::stream::test_support::{stream_inner, with_sent};
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Capture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("capture").extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn capturing() -> (Capture, tracing::subscriber::DefaultGuard) {
    let capture = Capture::default();
    let sink = capture.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .with_writer(move || sink.clone())
        .finish();
    (capture, tracing::subscriber::set_default(subscriber))
}

fn text(capture: &Capture) -> String {
    String::from_utf8(capture.0.lock().expect("capture").clone()).expect("utf8")
}

#[test]
fn rto_fire_is_logged_at_debug() {
    let (capture, _scope) = capturing();
    let mut inner = stream_inner();
    with_sent(&mut inner, 0, 3);

    inner.on_rto_fire_logged(7);

    let log = text(&capture);
    assert!(
        log.contains("stream rto fired")
            && log.contains("local_id=7")
            && log.contains("timeouts=1")
            && log.contains("unacked=3")
            && log.contains("rto_ms="),
        "log: {log}"
    );
}

#[test]
fn rto_fire_with_nothing_unacked_is_not_logged() {
    let (capture, _scope) = capturing();
    let mut inner = stream_inner();

    inner.on_rto_fire_logged(7);

    assert!(!text(&capture).contains("stream rto fired"));
}
