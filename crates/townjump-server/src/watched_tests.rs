// Tests for `Watched`: `on_close` once on EOF or drop; a drop is silent if the stream does not report `close` (a fake one in JS).
use super::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::AsyncReadExt;

fn counter() -> (Arc<AtomicUsize>, impl FnOnce() + Send + 'static) {
    let count = Arc::new(AtomicUsize::new(0));
    let hit = Arc::clone(&count);
    (count, move || {
        hit.fetch_add(1, Ordering::SeqCst);
    })
}

#[test]
fn drop_reports_close_when_the_stream_reports_it() {
    let (count, on_close) = counter();

    drop(Watched::new(tokio::io::duplex(8).0, true, on_close));

    assert_eq!(count.load(Ordering::SeqCst), 1);
}

#[test]
fn drop_stays_silent_when_the_stream_does_not_report_close() {
    let (count, on_close) = counter();

    drop(Watched::new(tokio::io::duplex(8).0, false, on_close));

    assert_eq!(count.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn eof_reports_close_once_even_for_a_silent_stream() {
    let (count, on_close) = counter();
    let (peer, ours) = tokio::io::duplex(8);
    let mut watched = Watched::new(ours, false, on_close);
    drop(peer);

    let mut rest = Vec::new();
    watched.read_to_end(&mut rest).await.unwrap();
    drop(watched);

    assert_eq!(count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn eof_then_drop_reports_close_only_once() {
    let (count, on_close) = counter();
    let (peer, ours) = tokio::io::duplex(8);
    let mut watched = Watched::new(ours, true, on_close);
    drop(peer);

    let mut rest = Vec::new();
    watched.read_to_end(&mut rest).await.unwrap();
    drop(watched);

    assert_eq!(count.load(Ordering::SeqCst), 1);
}
