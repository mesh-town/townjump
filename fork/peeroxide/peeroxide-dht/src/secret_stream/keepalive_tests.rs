// Tests for the keepalive and idle timeout of SecretStream (timers.rs) on a paused clock.
// The peer is a raw duplex end: it builds and reads frames byte by byte, with the keys of the
// JS probe (tests/node/secret-stream-keepalive.js): K1 = 0x11 x 32 (stream -> peer), K2 = 0x22 x 32.

use std::io::ErrorKind;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use tokio::time::Instant;

use super::error::SecretStreamError;
use super::frame::{IDHEADERBYTES, encode_frame, stream_id};
use super::session::SecretStream;
use super::test_support::{Probe, TestIo, session_pair};
use crate::noise;
use crate::secretstream::{Pull, Push};

const K1: [u8; 32] = [0x11; 32];
const K2: [u8; 32] = [0x22; 32];
const HASH: [u8; 64] = [0x33; 64];
const PUBLIC_KEY: [u8; 32] = [0xCC; 32];
/// Virtual clock rounds deadlines up to a millisecond; a few periods add a few of them.
const TOLERANCE: Duration = Duration::from_millis(50);

/// JS probe `sentA[0]`: `38 00 00`, 32-byte stream id, 24-byte secretstream header of K1.
const JS_ID_MESSAGE: &str = "38000007fa5856f1781e509fb6bfc4eb97ddb078fd276941f9b4176c1ff56dbc019388ca9e492ef3a1cbac39b642e089ea5320e1120de44d4dbd2d";
/// JS probe `sentA[1..]`: "x" written at 3036 ms, then keepalives at 8045 and 13058 ms.
const JS_FRAMES: [(&str, &[u8]); 3] = [
    ("120000d6ed1372ef4272fe47a99bb6a6bed736fb13", b"x"),
    ("110000fbeb8a9b9dac626f6f374b35267e498d21", b""),
    ("110000ea40367b31ccf9b2096a298e0fc854ffca", b""),
];

fn js_header() -> [u8; 24] {
    let message = hex::decode(JS_ID_MESSAGE).unwrap();
    message[3 + 32..].try_into().unwrap()
}

#[test]
fn pull_decrypts_the_frames_js_sent() {
    let mut pull = Pull::new(&K1, &js_header());
    for (frame_hex, expected) in JS_FRAMES {
        let frame = hex::decode(frame_hex).unwrap();
        let (plain, _tag) = pull.next(&frame[3..]).unwrap();
        assert_eq!(plain, expected);
    }
}

#[test]
fn push_writes_the_bytes_js_sent() {
    let mut push = Push::with_header(&K1, &js_header());
    for (frame_hex, plain) in JS_FRAMES {
        let frame = encode_frame(&push.next(plain)).unwrap();
        assert_eq!(hex::encode(frame), frame_hex);
    }
}

/// One frame the stream sent, decrypted, with the virtual time it was complete.
struct Seen {
    wire: Vec<u8>,
    plain: Vec<u8>,
    at: Duration,
}

fn assert_secs(seen: &Seen, secs: u64) {
    assert_at(seen.at, secs);
}

fn assert_at(at: Duration, secs: u64) {
    let want = Duration::from_secs(secs);
    assert!(
        at >= want && at < want + TOLERANCE,
        "at {at:?}, expected {secs} s"
    );
}

fn assert_keepalive_frame(seen: &Seen) {
    assert_eq!(seen.wire.len(), 20);
    assert_eq!(seen.wire[..3], [0x11, 0, 0]);
    assert!(seen.plain.is_empty());
}

fn assert_timed_out(err: SecretStreamError) {
    let SecretStreamError::Io(e) = err else {
        panic!("expected an I/O error, got {err:?}");
    };
    assert_eq!(e.kind(), ErrorKind::TimedOut);
    assert_eq!(e.to_string(), "Stream timed out");
}

/// The raw end of the connection: reads what the stream sends (key K1), writes with K2.
struct Peer {
    raw: DuplexStream,
    rx: Pull,
    tx: Push,
}

impl Peer {
    /// ID exchange as the responder: take the stream's header, send ours.
    async fn meet(mut raw: DuplexStream) -> Self {
        let mut id_message = [0u8; 3 + IDHEADERBYTES];
        raw.read_exact(&mut id_message).await.unwrap();
        let (tx, header) = Push::new(&K2);
        let mut ours = stream_id(&HASH, false).to_vec();
        ours.extend_from_slice(&header);
        raw.write_all(&encode_frame(&ours).unwrap()).await.unwrap();
        let rx = Pull::new(&K1, &id_message[3 + 32..].try_into().unwrap());
        Self { raw, rx, tx }
    }

    async fn send(&mut self, plain: &[u8]) {
        let frame = encode_frame(&self.tx.next(plain)).unwrap();
        self.raw.write_all(&frame).await.unwrap();
    }

    async fn next_frame(&mut self, t0: Instant) -> Seen {
        let mut wire = vec![0u8; 3];
        self.raw.read_exact(&mut wire).await.unwrap();
        let len = wire[0] as usize | (wire[1] as usize) << 8 | (wire[2] as usize) << 16;
        wire.resize(3 + len, 0);
        self.raw.read_exact(&mut wire[3..]).await.unwrap();
        let at = t0.elapsed();
        let (plain, _tag) = self.rx.next(&wire[3..]).unwrap();
        Seen { wire, plain, at }
    }
}

async fn stream_and_peer(capacity: usize) -> (SecretStream<DuplexStream>, Peer) {
    let (stream_io, peer_io) = tokio::io::duplex(capacity);
    let (stream, peer) = tokio::join!(
        SecretStream::from_session(true, stream_io, K1, K2, HASH, PUBLIC_KEY),
        Peer::meet(peer_io),
    );
    (stream.unwrap(), peer)
}

#[tokio::test(start_paused = true)]
async fn idle_stream_sends_one_keepalive_per_period() {
    let (mut a, mut peer) = stream_and_peer(8192).await;
    let t0 = Instant::now();
    let _reader = tokio::spawn(async move { a.read().await });
    for n in 1..=3 {
        // Two frames at one tick would put the next one at 5 s, not 10 s.
        let seen = peer.next_frame(t0).await;
        assert_secs(&seen, 5 * n);
        assert_keepalive_frame(&seen);
    }
}

#[tokio::test(start_paused = true)]
async fn set_keep_alive_changes_the_period() {
    let (mut a, mut peer) = stream_and_peer(8192).await;
    a.set_keep_alive(Duration::from_secs(2));
    let t0 = Instant::now();
    let _reader = tokio::spawn(async move { a.read().await });
    for n in 1..=2 {
        assert_secs(&peer.next_frame(t0).await, 2 * n);
    }
}

#[tokio::test(start_paused = true)]
async fn zero_keep_alive_is_silent() {
    let (mut a, mut peer) = stream_and_peer(8192).await;
    a.set_keep_alive(Duration::ZERO);
    let reader = tokio::spawn(async move { a.read().await });
    tokio::time::sleep(Duration::from_secs(60)).await;
    let mut byte = [0u8; 1];
    let read = tokio::time::timeout(Duration::from_secs(1), peer.raw.read_exact(&mut byte));
    assert!(read.await.is_err(), "the stream sent bytes");
    assert!(!reader.is_finished());
}

#[tokio::test(start_paused = true)]
async fn write_at_3s_moves_the_next_keepalive_to_8s() {
    let (a, mut peer) = stream_and_peer(8192).await;
    let t0 = Instant::now();
    let (mut read_half, mut write_half) = tokio::io::split(a);
    let _reader = tokio::spawn(async move { read_half.read(&mut [0u8; 16]).await });
    tokio::time::sleep(Duration::from_secs(3)).await;
    write_half.write_all(b"x").await.unwrap();
    let data = peer.next_frame(t0).await;
    assert_secs(&data, 3);
    assert_eq!(
        (data.wire[..3].to_vec(), data.plain),
        (vec![0x12, 0, 0], b"x".to_vec())
    );
    let keepalive = peer.next_frame(t0).await;
    assert_secs(&keepalive, 8);
    assert_keepalive_frame(&keepalive);
}

#[tokio::test(start_paused = true)]
async fn write_call_moves_the_next_keepalive_and_read_survives_the_drop() {
    let (mut a, mut peer) = stream_and_peer(8192).await;
    let t0 = Instant::now();
    let _task = tokio::spawn(async move {
        let _ = tokio::time::timeout(Duration::from_secs(3), a.read()).await;
        a.write(b"x").await.unwrap();
        a.read().await
    });
    assert_secs(&peer.next_frame(t0).await, 3);
    let keepalive = peer.next_frame(t0).await;
    assert_secs(&keepalive, 8);
    assert_keepalive_frame(&keepalive);
}

#[tokio::test(start_paused = true)]
async fn keepalive_follows_a_half_written_frame_intact() {
    let (a, mut peer) = stream_and_peer(64).await;
    let t0 = Instant::now();
    let (mut read_half, mut write_half) = tokio::io::split(a);
    let _reader = tokio::spawn(async move { read_half.read(&mut [0u8; 16]).await });
    write_half.write_all(&[7u8; 1000]).await.unwrap();
    let data = peer.next_frame(t0).await;
    assert_eq!(data.plain, vec![7u8; 1000]);
    // The nonce counter went on: the keepalive behind it still decrypts.
    assert_keepalive_frame(&peer.next_frame(t0).await);
}

#[tokio::test(start_paused = true)]
async fn end_of_our_side_stops_the_keepalive() {
    let ((mut a, probe), (_b, _)) = session_pair(8192).await;
    a.shutdown().await.unwrap();
    let writes = probe.writes.load(Ordering::SeqCst);
    let reader = tokio::spawn(async move { a.read().await });
    tokio::time::sleep(Duration::from_secs(12)).await;
    assert_eq!(probe.writes.load(Ordering::SeqCst), writes);
    assert!(!reader.is_finished());
}

async fn assert_one_keepalive_write_in_5s(mut a: SecretStream<TestIo>, probe: Arc<Probe>) {
    let writes = probe.writes.load(Ordering::SeqCst);
    let _reader = tokio::spawn(async move { a.read().await });
    tokio::time::sleep(Duration::from_millis(5500)).await;
    assert_eq!(probe.writes.load(Ordering::SeqCst), writes + 1);
}

#[tokio::test(start_paused = true)]
async fn from_session_stream_has_the_default_keep_alive() {
    let ((a, probe), (_b, _)) = session_pair(8192).await;
    assert_one_keepalive_write_in_5s(a, probe).await;
}

#[tokio::test(start_paused = true)]
async fn handshaken_stream_has_the_default_keep_alive() {
    let ((a_io, probe), (b_io, _)) = TestIo::pair(8192);
    let (a, b) = tokio::join!(
        SecretStream::new(true, a_io, noise::generate_keypair()),
        SecretStream::new(false, b_io, noise::generate_keypair()),
    );
    let _b = b.unwrap();
    assert_one_keepalive_write_in_5s(a.unwrap(), probe).await;
}

#[tokio::test(start_paused = true)]
async fn silent_peer_times_out_after_the_timeout() {
    let (mut a, _peer) = stream_and_peer(8192).await;
    a.set_timeout(Duration::from_secs(2));
    let t0 = Instant::now();
    assert_timed_out(a.read().await.unwrap_err());
    assert_at(t0.elapsed(), 2);
}

#[tokio::test(start_paused = true)]
async fn incoming_bytes_at_1s_move_the_timeout_to_3s() {
    let (mut a, mut peer) = stream_and_peer(8192).await;
    a.set_timeout(Duration::from_secs(2));
    let t0 = Instant::now();
    let sender = async {
        tokio::time::sleep(Duration::from_secs(1)).await;
        peer.send(b"hi").await;
    };
    let reader = async {
        assert_eq!(a.read().await.unwrap().unwrap(), b"hi");
        assert_at(t0.elapsed(), 1);
        assert_timed_out(a.read().await.unwrap_err());
        assert_at(t0.elapsed(), 3);
    };
    tokio::join!(sender, reader);
}

#[tokio::test(start_paused = true)]
async fn async_read_fails_with_timed_out() {
    let (mut a, _peer) = stream_and_peer(8192).await;
    a.set_timeout(Duration::from_secs(2));
    let err = AsyncReadExt::read(&mut a, &mut [0u8; 8]).await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::TimedOut);
    assert_eq!(err.to_string(), "Stream timed out");
}

#[tokio::test(start_paused = true)]
async fn no_timeout_by_default() {
    let (mut a, _peer) = stream_and_peer(8192).await;
    let waited = tokio::time::timeout(Duration::from_secs(600), a.read()).await;
    assert!(waited.is_err(), "read ended without data: {waited:?}");
}

#[tokio::test(start_paused = true)]
async fn read_skips_empty_messages_and_never_returns_them() {
    let (mut a, mut peer) = stream_and_peer(8192).await;
    peer.send(b"").await;
    peer.send(b"").await;
    peer.send(b"data").await;
    assert_eq!(a.read().await.unwrap().unwrap(), b"data");
    peer.send(b"").await;
    drop(peer);
    assert!(a.read().await.unwrap().is_none());
}
