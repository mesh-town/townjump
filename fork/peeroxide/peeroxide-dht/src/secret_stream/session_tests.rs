// Tests for the handshake, from_session and the split halves.

use std::sync::atomic::Ordering::SeqCst;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::timeout;

use super::SecretStream;
use crate::noise;
use crate::secret_stream::test_support::{TestIo, session_pair_held_flush};

#[tokio::test]
async fn handshake_and_exchange() {
    // Full in-process test using duplex streams (tokio::io::duplex).
    let (client_stream, server_stream) = tokio::io::duplex(8192);

    let kp_a = noise::generate_keypair();
    let kp_b = noise::generate_keypair();

    let (client_result, server_result) = tokio::join!(
        SecretStream::new(true, client_stream, kp_a),
        SecretStream::new(false, server_stream, kp_b),
    );

    let mut client = client_result.expect("client handshake failed");
    let mut server = server_result.expect("server handshake failed");

    // Send from client → server
    client.write(b"hello from client").await.unwrap();
    let msg = server.read().await.unwrap().expect("expected message");
    assert_eq!(msg, b"hello from client");

    // Send from server → client
    server.write(b"hello from server").await.unwrap();
    let msg = client.read().await.unwrap().expect("expected message");
    assert_eq!(msg, b"hello from server");

    // Multiple messages
    for i in 0..10 {
        let payload = format!("message {i}");
        client.write(payload.as_bytes()).await.unwrap();
        let msg = server.read().await.unwrap().expect("expected message");
        assert_eq!(msg, payload.as_bytes());
    }
}

#[tokio::test]
async fn empty_message_roundtrip() {
    let (client_stream, server_stream) = tokio::io::duplex(8192);

    let (mut client, mut server) = tokio::try_join!(
        SecretStream::new(true, client_stream, noise::generate_keypair()),
        SecretStream::new(false, server_stream, noise::generate_keypair()),
    )
    .unwrap();

    // Empty messages are consumed as keepalives by read().
    // Send a real message after an empty one.
    client.write(b"").await.unwrap();
    client.write(b"after empty").await.unwrap();
    let msg = server.read().await.unwrap().expect("expected message");
    assert_eq!(msg, b"after empty");
}

#[tokio::test]
async fn remote_public_key_matches() {
    let (client_stream, server_stream) = tokio::io::duplex(8192);

    let kp_a = noise::generate_keypair();
    let kp_b = noise::generate_keypair();
    let pk_a = kp_a.public_key;
    let pk_b = kp_b.public_key;

    let (client, server) = tokio::try_join!(
        SecretStream::new(true, client_stream, kp_a),
        SecretStream::new(false, server_stream, kp_b),
    )
    .unwrap();

    assert_eq!(*client.remote_public_key(), pk_b);
    assert_eq!(*server.remote_public_key(), pk_a);
    assert_eq!(client.handshake_hash(), server.handshake_hash());
}

#[tokio::test]
async fn from_session_roundtrip() {
    let (client_stream, server_stream) = tokio::io::duplex(8192);

    let tx_key = [0x11u8; 32];
    let rx_key = [0x22u8; 32];
    let hash = [0xABu8; 64];
    let remote_pk = [0xCCu8; 32];

    let (client_result, server_result) = tokio::join!(
        SecretStream::from_session(true, client_stream, tx_key, rx_key, hash, remote_pk),
        SecretStream::from_session(false, server_stream, rx_key, tx_key, hash, remote_pk),
    );

    let mut client = client_result.expect("client from_session failed");
    let mut server = server_result.expect("server from_session failed");

    client.write(b"pre-keyed hello").await.unwrap();
    let msg = server.read().await.unwrap().expect("expected message");
    assert_eq!(msg, b"pre-keyed hello");

    server.write(b"pre-keyed reply").await.unwrap();
    let msg = client.read().await.unwrap().expect("expected message");
    assert_eq!(msg, b"pre-keyed reply");

    assert!(client.is_initiator());
    assert!(!server.is_initiator());
}

#[tokio::test]
async fn into_split_roundtrip() {
    // Split both peers into read/write halves and exchange in both
    // directions concurrently. Proves the split preserves wire framing and
    // AEAD cipher behavior identically to the unsplit read()/write().
    let (client_stream, server_stream) = tokio::io::duplex(8192);

    let (client, server) = tokio::try_join!(
        SecretStream::new(true, client_stream, noise::generate_keypair()),
        SecretStream::new(false, server_stream, noise::generate_keypair()),
    )
    .unwrap();

    let (mut client_rd, mut client_wr) = client.into_split();
    let (mut server_rd, mut server_wr) = server.into_split();

    // client → server
    client_wr.write(b"hello from client").await.unwrap();
    let msg = server_rd.read().await.unwrap().expect("expected message");
    assert_eq!(msg, b"hello from client");

    // server → client
    server_wr.write(b"hello from server").await.unwrap();
    let msg = client_rd.read().await.unwrap().expect("expected message");
    assert_eq!(msg, b"hello from server");

    // Concurrent bidirectional bulk: writer and reader in separate tasks,
    // read_exact is never cancelled — the exact property the split provides.
    let writer = tokio::spawn(async move {
        for i in 0..100 {
            let payload = format!("bulk {i}");
            client_wr.write(payload.as_bytes()).await.unwrap();
        }
        client_wr.shutdown().await.unwrap();
    });
    let reader = tokio::spawn(async move {
        let mut count = 0;
        while let Some(msg) = server_rd.read().await.unwrap() {
            assert_eq!(msg, format!("bulk {count}").as_bytes());
            count += 1;
        }
        count
    });
    writer.await.unwrap();
    assert_eq!(reader.await.unwrap(), 100);
}

#[tokio::test]
async fn into_split_empty_keepalive() {
    // Empty messages are consumed as keepalives by the split read half,
    // matching SecretStream::read.
    let (client_stream, server_stream) = tokio::io::duplex(8192);

    let (client, server) = tokio::try_join!(
        SecretStream::new(true, client_stream, noise::generate_keypair()),
        SecretStream::new(false, server_stream, noise::generate_keypair()),
    )
    .unwrap();

    let (_client_rd, mut client_wr) = client.into_split();
    let (mut server_rd, _server_wr) = server.into_split();

    client_wr.write(b"").await.unwrap();
    client_wr.write(b"after empty").await.unwrap();
    let msg = server_rd.read().await.unwrap().expect("expected message");
    assert_eq!(msg, b"after empty");
}

/// The ID header goes out without waiting for its ACK (UDX `flush` is pending until the
/// peer's ACK), so `from_session` finishes while the ACK is outstanding and reads the peer's header.
#[tokio::test]
async fn from_session_does_not_wait_for_the_header_ack() {
    let pair = timeout(Duration::from_secs(2), session_pair_held_flush(8192)).await;
    let ((mut a, a_probe), (mut b, b_probe)) =
        pair.expect("from_session waited for the ACK of its own header");
    assert_eq!(
        (a_probe.writes.load(SeqCst), a_probe.flushes.load(SeqCst)),
        (1, 0)
    );
    assert_eq!(
        (b_probe.writes.load(SeqCst), b_probe.flushes.load(SeqCst)),
        (1, 0)
    );

    // The application writes through `AsyncWrite` (no flush): data follow the header, ACK still out.
    timeout(Duration::from_secs(2), a.write_all(b"while held"))
        .await
        .expect("write stuck behind the unacknowledged header")
        .unwrap();
    let mut buf = [0u8; 10];
    timeout(Duration::from_secs(2), b.read_exact(&mut buf))
        .await
        .expect("data after the header did not arrive")
        .unwrap();
    assert_eq!(&buf, b"while held");

    // The ACK arrives; framed writes (with flush) keep their order in both directions.
    a_probe.release_flush();
    b_probe.release_flush();
    a.write(b"after header").await.unwrap();
    assert_eq!(b.read().await.unwrap().unwrap(), b"after header");
    b.write(b"reply").await.unwrap();
    assert_eq!(a.read().await.unwrap().unwrap(), b"reply");
}

/// Noise path: only the three handshake messages are flushed, the ID header is not
/// (initiator M1 and M3, responder M2).
#[tokio::test]
async fn handshake_does_not_flush_the_header() {
    let ((a_io, a_probe), (b_io, b_probe)) = TestIo::pair(8192);
    let (a, b) = tokio::join!(
        SecretStream::new(true, a_io, noise::generate_keypair()),
        SecretStream::new(false, b_io, noise::generate_keypair()),
    );
    a.expect("initiator handshake failed");
    b.expect("responder handshake failed");
    assert_eq!(a_probe.flushes.load(SeqCst), 2);
    assert_eq!(b_probe.flushes.load(SeqCst), 1);
}
