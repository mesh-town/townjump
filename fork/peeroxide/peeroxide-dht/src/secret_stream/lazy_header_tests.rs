// `from_session` sends its own ID header and returns; the peer's header is read at the first read.

use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::time::timeout;

use super::{SecretStream, SecretStreamError};
use crate::secret_stream::frame::write_frame;

const TX: [u8; 32] = [0x11; 32];
const RX: [u8; 32] = [0x22; 32];
const HASH: [u8; 64] = [0xAB; 64];
const PK: [u8; 32] = [0xCC; 32];

/// (a) The peer is silent: `from_session` still returns, and the first read waits for the header.
#[tokio::test]
async fn from_session_returns_while_the_peer_is_silent() {
    let (client, _silent_peer) = tokio::io::duplex(8192);
    let mut ss = timeout(
        Duration::from_secs(2),
        SecretStream::from_session(true, client, TX, RX, HASH, PK),
    )
    .await
    .expect("from_session waited for the peer's header")
    .expect("from_session failed");
    assert!(
        timeout(Duration::from_millis(100), ss.read())
            .await
            .is_err(),
        "read must wait for the peer's header"
    );
}

/// (b) A bad peer header fails the first read, not `from_session`.
#[tokio::test]
async fn bad_peer_header_fails_the_first_read() {
    let (client, mut peer) = tokio::io::duplex(8192);
    write_frame(&mut peer, &[0u8; 10]).await.unwrap();
    peer.flush().await.unwrap();
    let mut ss = SecretStream::from_session(true, client, TX, RX, HASH, PK)
        .await
        .expect("a bad peer header must not fail from_session");
    assert!(matches!(
        ss.read().await,
        Err(SecretStreamError::InvalidIdHeader {
            expected: 56,
            got: 10
        })
    ));
}

/// (b) A header with a foreign stream id is a mismatch on the first read.
#[tokio::test]
async fn foreign_stream_id_fails_the_first_read() {
    let (client, mut peer) = tokio::io::duplex(8192);
    write_frame(&mut peer, &[7u8; 56]).await.unwrap();
    let mut ss = SecretStream::from_session(true, client, TX, RX, HASH, PK)
        .await
        .unwrap();
    assert!(matches!(
        ss.read().await,
        Err(SecretStreamError::StreamIdMismatch)
    ));
}

/// (b) Peer closes without a header: the first read reports the EOF.
#[tokio::test]
async fn closed_peer_fails_the_first_read() {
    let (client, mut peer) = tokio::io::duplex(8192);
    peer.shutdown().await.unwrap(); // EOF for the client; its own header still goes out
    let mut ss = SecretStream::from_session(true, client, TX, RX, HASH, PK)
        .await
        .unwrap();
    assert!(ss.read().await.is_err());
}

/// The lazy header also works through `into_split` before any read.
#[tokio::test]
async fn split_before_first_read() {
    let (c, s) = tokio::io::duplex(8192);
    let (a, b) = tokio::join!(
        SecretStream::from_session(true, c, TX, RX, HASH, PK),
        SecretStream::from_session(false, s, RX, TX, HASH, PK),
    );
    let (mut ar, _aw) = a.unwrap().into_split();
    let (_br, mut bw) = b.unwrap().into_split();
    bw.write(b"late").await.unwrap();
    assert_eq!(ar.read().await.unwrap().unwrap(), b"late");
}
