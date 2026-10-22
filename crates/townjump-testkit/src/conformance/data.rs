// JS: streamx Duplex of a hyperdht connection (`write`, `end`, `on('data')`); a byte pattern instead of random data.
// Related: wait.rs (wait limits), stream.rs and one_key.rs (scenarios that push the bytes).
use super::wait::{STREAM_WAIT, within};
use tokio::io::{AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf, split};
use townjump_dht::Duplex;

/// Pattern salt from client to server and back: the directions differ by content.
const UP_SALT: usize = 1;
const DOWN_SALT: usize = 2;

/// Deterministic bytes: `(i * 31 + salt) % 251`; the period 251 does not divide the block sizes (powers of two).
pub fn pattern(len: usize, salt: usize) -> Vec<u8> {
    (0..len).map(|i| ((i * 31 + salt) % 251) as u8).collect()
}

/// Bytes are equal; on a mismatch the text names the first differing position instead of printing a megabyte.
pub fn assert_same(got: &[u8], want: &[u8], what: &str) {
    assert_eq!(got.len(), want.len(), "{what}: length mismatch");
    let first = got.iter().zip(want).position(|(a, b)| a != b);
    assert!(
        first.is_none(),
        "{what}: bytes diverge from position {first:?}"
    );
}

/// Writes `bytes`, flushes the buffer and closes its own write side (`END`).
pub async fn send_and_close(stream: &mut Box<dyn Duplex>, bytes: &[u8]) {
    stream.write_all(bytes).await.expect("write_all");
    stream.flush().await.expect("flush");
    stream.shutdown().await.expect("shutdown");
}

/// Reads the stream until `EOF`.
pub async fn read_all(stream: &mut Box<dyn Duplex>, what: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    let read = within(STREAM_WAIT, what, stream.read_to_end(&mut bytes)).await;
    read.unwrap_or_else(|e| panic!("{what}: read failed: {e}"));
    bytes
}

async fn write_flush(mut half: WriteHalf<Box<dyn Duplex>>, bytes: &[u8]) {
    half.write_all(bytes).await.expect("write_all");
    half.flush().await.expect("flush");
}

async fn read_len(mut half: ReadHalf<Box<dyn Duplex>>, len: usize) -> Vec<u8> {
    let mut bytes = vec![0; len];
    half.read_exact(&mut bytes).await.expect("read_exact");
    bytes
}

/// Both sides write `len` pattern bytes at once and read `len` bytes from the other side; the bytes are equal.
pub async fn exchange(client: Box<dyn Duplex>, server: Box<dyn Duplex>, len: usize, what: &str) {
    let (client_read, client_write) = split(client);
    let (server_read, server_write) = split(server);
    let (up, down) = (pattern(len, UP_SALT), pattern(len, DOWN_SALT));
    let both = async {
        tokio::join!(
            write_flush(client_write, &up),
            write_flush(server_write, &down),
            read_len(server_read, len),
            read_len(client_read, len),
        )
    };
    let (_, _, server_got, client_got) = within(STREAM_WAIT, what, both).await;
    assert_same(&server_got, &up, &format!("{what}: client → server"));
    assert_same(&client_got, &down, &format!("{what}: server → client"));
}

#[cfg(test)]
#[path = "data_tests.rs"]
mod tests;
