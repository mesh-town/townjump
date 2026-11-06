// A keepalive sent from the reading task must not strand a writer that waits in `flush`.

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::test_support::session_pair;

#[tokio::test(start_paused = true)]
async fn keepalive_from_reader_does_not_strand_flushing_writer() {
    let ((a, probe), (_b, _)) = session_pair(8192).await;
    let (mut read_half, mut write_half) = tokio::io::split(a);
    probe.hold_flush();
    let (done_tx, done_rx) = tokio::sync::oneshot::channel();
    let _writer = tokio::spawn(async move {
        write_half.write_all(b"x").await.unwrap();
        write_half.flush().await.unwrap();
        let _ = done_tx.send(());
    });
    let _reader = tokio::spawn(async move { read_half.read(&mut [0u8; 16]).await });
    // Past the 5 s keepalive period: the keepalive polls the transport from the reader's task.
    tokio::time::sleep(Duration::from_secs(6)).await;
    probe.release_flush();
    let flushed = tokio::time::timeout(Duration::from_secs(1), done_rx).await;
    assert!(flushed.is_ok(), "the writer's flush was not woken");
}
