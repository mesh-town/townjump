// Tests for framing and stream-ID derivation.

use std::io::ErrorKind;
use std::sync::atomic::Ordering;

use tokio::io::AsyncReadExt;

use super::{NS_SECRET_STREAM, read_frame, stream_id, write_frame};
use crate::secret_stream::SecretStreamError;
use crate::secret_stream::test_support::TestIo;

#[test]
fn namespace_constants_are_distinct() {
    let ns = &*NS_SECRET_STREAM;
    assert_ne!(ns[0], ns[1]);
    assert_ne!(ns[1], ns[2]);
    assert_ne!(ns[0], ns[2]);
}

#[test]
fn stream_id_differs_by_role() {
    let hash = [0xABu8; 64];
    let id_i = stream_id(&hash, true);
    let id_r = stream_id(&hash, false);
    assert_ne!(id_i, id_r);
}

#[test]
fn frame_roundtrip() {
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let data = b"hello framing";
        let mut buf = Vec::new();
        write_frame(&mut buf, data).await.unwrap();

        assert_eq!(buf.len(), 3 + data.len());
        assert_eq!(buf[0], data.len() as u8);
        assert_eq!(buf[1], 0);
        assert_eq!(buf[2], 0);

        let mut cursor = std::io::Cursor::new(buf);
        let result = read_frame(&mut cursor).await.unwrap();
        assert_eq!(result.as_deref(), Some(data.as_slice()));
    });
}

#[test]
fn frame_eof_returns_none() {
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let mut cursor = std::io::Cursor::new(Vec::<u8>::new());
        let result = read_frame(&mut cursor).await.unwrap();
        assert!(result.is_none());
    });
}

#[tokio::test]
async fn write_frame_is_one_write_call() {
    let ((mut a, probe), _peer) = TestIo::pair(8192);
    write_frame(&mut a, &[7u8; 100]).await.unwrap();

    assert_eq!(probe.writes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn read_frame_body_error_is_io() {
    let ((failing, probe), _peer) = TestIo::pair(64);
    probe.fail_reads.store(true, Ordering::SeqCst);
    // Header says 5 body bytes; the body read then hits the reset.
    let mut reader = std::io::Cursor::new(vec![5u8, 0, 0]).chain(failing);

    let err = read_frame(&mut reader).await.unwrap_err();
    assert!(matches!(&err, SecretStreamError::Io(e) if e.kind() == ErrorKind::ConnectionReset));
}
