use super::*;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn write_is_swallowed() {
    let mut stream = StubStream;
    stream.write_all(b"data").await.unwrap();
    stream.flush().await.unwrap();
}

#[tokio::test]
async fn read_is_never_ready() {
    let mut stream = StubStream;
    let mut buf = [0u8; 4];
    let read = tokio::time::timeout(Duration::from_millis(50), stream.read(&mut buf)).await;
    assert!(read.is_err(), "read from the stub returned: {read:?}");
}

#[tokio::test]
async fn shutdown_is_ok() {
    StubStream.shutdown().await.unwrap();
}
