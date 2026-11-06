//! The debug line with the read-path counters printed when an adapter is dropped.

use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::native::runtime::UdxRuntime;
use crate::native::stream::test_support::bound_socket;

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

#[tokio::test(start_paused = true)]
async fn read_stats_are_logged_on_drop() {
    let capture = Capture::default();
    let sink = capture.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .with_writer(move || sink.clone())
        .finish();
    let _scope = tracing::subscriber::set_default(subscriber);

    let runtime = UdxRuntime::new().expect("runtime");
    let (sock_a, addr_a) = bound_socket(&runtime).await;
    let (sock_b, addr_b) = bound_socket(&runtime).await;
    let a = runtime.create_stream(1).await.expect("stream a");
    let b = runtime.create_stream(100).await.expect("stream b");
    a.connect(&sock_a, 100, addr_b).await.expect("connect a");
    b.connect(&sock_b, 1, addr_a).await.expect("connect b");
    let mut writer = a.into_async_stream();
    let mut reader = b.into_async_stream();

    writer.write_all(&[7u8; 1000]).await.expect("write");
    let mut got = [0u8; 1000];
    reader.read_exact(&mut got).await.expect("read");
    // Other tests hit this callsite without a subscriber; drop any cached "disabled" state.
    tracing::callsite::rebuild_interest_cache();
    drop(reader);

    let log = String::from_utf8(capture.0.lock().expect("capture").clone()).expect("utf8");
    assert!(
        log.contains("stream read stats")
            && log.contains("local_id=100")
            && log.contains("delivered=1000")
            && log.contains("taken=1000")
            && log.contains("last_read_pending=false"),
        "log: {log}"
    );
    drop(writer);
}
