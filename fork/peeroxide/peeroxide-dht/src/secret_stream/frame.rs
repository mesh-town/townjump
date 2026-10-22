//! Length-prefixed framing, stream-ID derivation and protocol constants.
// JS: @hyperswarm/secret-stream/index.js

use std::sync::LazyLock;

use blake2::Blake2bMac;
use blake2::digest::consts::U32;
use blake2::digest::{KeyInit, Mac};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use super::error::SecretStreamError;
use crate::crypto;
use crate::secretstream::HEADERBYTES;

pub(super) type Blake2bMac256 = Blake2bMac<U32>;

// ── Constants ────────────────────────────────────────────────────────────────

/// Size of the ID header body: 32-byte stream ID + 24-byte secretstream header.
pub(super) const IDHEADERBYTES: usize = HEADERBYTES + 32;

/// Maximum message body that fits in a 3-byte LE length prefix.
pub(super) const MAX_MESSAGE_LEN: usize = 0xFF_FF_FF; // 16,777,215

/// Namespace constants for `hyperswarm/secret-stream`.
/// Index 0 = initiator, 1 = responder, 2 = send (unordered, unused for now).
pub(super) static NS_SECRET_STREAM: LazyLock<[[u8; 32]; 3]> = LazyLock::new(|| {
    let ns = crypto::namespace("hyperswarm/secret-stream", &[0, 1, 2]);
    [ns[0], ns[1], ns[2]]
});

pub(super) fn ns_initiator() -> &'static [u8; 32] {
    &NS_SECRET_STREAM[0]
}
pub(super) fn ns_responder() -> &'static [u8; 32] {
    &NS_SECRET_STREAM[1]
}

// ── Framing helpers ──────────────────────────────────────────────────────────

/// Build one length-prefixed frame `[3B LE uint24 length][body]` in a single buffer.
pub(super) fn encode_frame(ciphertext: &[u8]) -> Result<Vec<u8>, SecretStreamError> {
    let len = ciphertext.len();
    if len > MAX_MESSAGE_LEN {
        return Err(SecretStreamError::MessageTooLarge { len });
    }
    let mut frame = Vec::with_capacity(3 + len);
    frame.extend_from_slice(&[len as u8, (len >> 8) as u8, (len >> 16) as u8]);
    frame.extend_from_slice(ciphertext);
    Ok(frame)
}

/// Write a length-prefixed frame with one `write_all` (one transport write), without flushing.
/// For the ID header: on UDX `flush` waits for the peer's ACK, and JS writes the header without
/// waiting (`secret-stream/index.js` `_setupSecretStream`: `rawStream.write(buf)`).
pub(super) async fn write_frame_unflushed<W: AsyncWrite + Unpin>(
    w: &mut W,
    data: &[u8],
) -> Result<(), SecretStreamError> {
    let frame = encode_frame(data)?;
    w.write_all(&frame).await?;
    Ok(())
}

/// Write a length-prefixed frame with one `write_all` (one transport write), then flush. On UDX the flush
/// waits for the peer's ACK of everything in flight (at least one RTT).
pub(super) async fn write_frame<W: AsyncWrite + Unpin>(
    w: &mut W,
    data: &[u8],
) -> Result<(), SecretStreamError> {
    write_frame_unflushed(w, data).await?;
    w.flush().await?;
    Ok(())
}

/// Map a body-read failure: EOF stays the handshake-style `UnexpectedEof`,
/// any other transport error (e.g. a reset) is kept as `Io`.
fn body_error(e: std::io::Error) -> SecretStreamError {
    if e.kind() == std::io::ErrorKind::UnexpectedEof {
        return SecretStreamError::UnexpectedEof;
    }
    SecretStreamError::Io(e)
}

/// Read a length-prefixed frame.
///
/// Returns `Ok(None)` on clean EOF (zero bytes read for the length header).
/// Returns `Err(UnexpectedEof)` if EOF occurs mid-message; any other body
/// read error is returned as `Err(Io)`.
pub(super) async fn read_frame<R: AsyncRead + Unpin>(
    r: &mut R,
) -> Result<Option<Vec<u8>>, SecretStreamError> {
    let mut header = [0u8; 3];
    match r.read_exact(&mut header).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(SecretStreamError::Io(e)),
    }

    let len = header[0] as usize | (header[1] as usize) << 8 | (header[2] as usize) << 16;
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).await.map_err(body_error)?;

    Ok(Some(buf))
}

// ── Stream ID derivation ─────────────────────────────────────────────────────

/// Compute the 32-byte stream identity for one side of the connection.
///
/// Equivalent to Node.js:
/// ```js
/// sodium.crypto_generichash(out, isInitiator ? NS_INITIATOR : NS_RESPONDER, handshakeHash)
/// ```
///
/// This is a keyed BLAKE2b-256: data = namespace constant, key = handshake hash.
pub(super) fn stream_id(handshake_hash: &[u8; 64], is_initiator: bool) -> [u8; 32] {
    let ns = if is_initiator {
        ns_initiator()
    } else {
        ns_responder()
    };
    let mut mac: Blake2bMac256 =
        // SAFETY: BLAKE2b accepts keys from 1..=64 bytes; handshake_hash is always 64 bytes.
        KeyInit::new_from_slice(handshake_hash).expect("64-byte key is valid for BLAKE2b");
    mac.update(ns);
    let output = mac.finalize().into_bytes();
    let mut result = [0u8; 32];
    result.copy_from_slice(&output);
    result
}

#[cfg(test)]
#[path = "frame_tests.rs"]
mod tests;
