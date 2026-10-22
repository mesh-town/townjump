//! Errors returned by `SecretStream` and its framing helpers.
// JS: @hyperswarm/secret-stream/index.js

use thiserror::Error;

use super::frame::MAX_MESSAGE_LEN;
use crate::noise::NoiseError;
use crate::secretstream;

#[derive(Debug, Error)]
/// Errors returned by `SecretStream` and its framing helpers.
#[non_exhaustive]
pub enum SecretStreamError {
    /// An underlying I/O operation failed.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Noise handshake negotiation failed.
    #[error("noise handshake failed: {0}")]
    Noise(#[from] NoiseError),

    /// Secretstream decryption failed.
    #[error("secretstream decryption failed: {0}")]
    Decrypt(#[from] secretstream::SecretstreamError),

    /// The handshake ended before completion.
    #[error("handshake did not complete")]
    HandshakeIncomplete,

    /// EOF was encountered while the handshake was still in progress.
    #[error("unexpected EOF during handshake")]
    UnexpectedEof,

    /// The ID header length was invalid.
    #[error("invalid ID header: expected {expected} bytes, got {got}")]
    InvalidIdHeader {
        /// The expected header length in bytes.
        expected: usize,
        /// The actual header length received.
        got: usize,
    },

    /// The remote stream ID did not match the expected identity.
    #[error("stream ID mismatch: remote peer sent wrong identity")]
    StreamIdMismatch,

    /// The framed message exceeded the maximum allowed length.
    #[error("message too large: {len} bytes exceeds maximum {MAX_MESSAGE_LEN}")]
    MessageTooLarge {
        /// The length of the oversized message in bytes.
        len: usize,
    },
}
