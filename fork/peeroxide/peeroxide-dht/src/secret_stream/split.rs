//! Owned read/write halves of a `SecretStream` and `SecretStream::into_split`.
// JS: @hyperswarm/secret-stream/index.js

use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, ReadHalf, WriteHalf};

use super::error::SecretStreamError;
use super::frame::{read_frame, write_frame};
use super::session::{PendingHeader, SecretStream};
use crate::secretstream::{self, ABYTES};

impl<T: AsyncRead + AsyncWrite + Unpin + Send> SecretStream<T> {
    /// Split the stream into owned read and write halves.
    ///
    /// The underlying transport is split with [`tokio::io::split`]; the
    /// decryptor (`Pull`) moves into the [`SecretStreamReadHalf`] and the
    /// encryptor (`Push`) moves into the [`SecretStreamWriteHalf`]. Because the
    /// two halves own disjoint cipher state, they can be driven from separate
    /// tasks concurrently without either half cancelling the other's in-flight
    /// framed read/write. Wire framing and cipher behavior are identical to the
    /// unsplit [`SecretStream::read`]/[`SecretStream::write`].
    ///
    /// The halves carry no keepalive and no idle timeout: the timers of
    /// [`SecretStream::set_keep_alive`]/[`SecretStream::set_timeout`] stay behind.
    pub fn into_split(
        self,
    ) -> (
        SecretStreamReadHalf<ReadHalf<T>>,
        SecretStreamWriteHalf<WriteHalf<T>>,
    ) {
        let (read_raw, write_raw) = tokio::io::split(self.raw);
        (
            SecretStreamReadHalf {
                raw: read_raw,
                decrypt: self.decrypt,
                pending_header: self.pending_header,
            },
            SecretStreamWriteHalf {
                raw: write_raw,
                encrypt: self.encrypt,
            },
        )
    }
}

/// Owned read half of a [`SecretStream`], produced by [`SecretStream::into_split`].
///
/// Holds the decryptor and the read side of the underlying transport. Its
/// [`read`](SecretStreamReadHalf::read) mirrors [`SecretStream::read`] exactly
/// (same length-prefixed framing, same empty-keepalive handling, same AEAD
/// decryption).
pub struct SecretStreamReadHalf<R> {
    raw: R,
    decrypt: Option<secretstream::Pull>,
    pending_header: Option<PendingHeader>,
}

impl<R: AsyncRead + Unpin + Send> SecretStreamReadHalf<R> {
    /// Read and decrypt the next framed message.
    ///
    /// Returns `Ok(None)` on clean EOF, `Ok(Some(plaintext))` for data
    /// messages. Empty messages (keepalives) are silently consumed and the next
    /// message is read. Identical framing/cipher semantics to
    /// [`SecretStream::read`].
    pub async fn read(&mut self) -> Result<Option<Vec<u8>>, SecretStreamError> {
        if let Some(pending) = &self.pending_header {
            let msg = read_frame(&mut self.raw)
                .await?
                .ok_or(SecretStreamError::UnexpectedEof)?;
            self.decrypt = Some(pending.accept(&msg)?);
            self.pending_header = None;
        }
        loop {
            let msg = match read_frame(&mut self.raw).await? {
                Some(m) => m,
                None => return Ok(None),
            };

            if msg.len() < ABYTES {
                return Err(SecretStreamError::Decrypt(
                    secretstream::SecretstreamError::CiphertextTooShort,
                ));
            }

            let decrypt = self
                .decrypt
                .as_mut()
                .ok_or(SecretStreamError::HandshakeIncomplete)?;
            let (plaintext, _tag) = decrypt.next(&msg)?;

            // Drop empty keepalive messages (match Node.js behavior).
            if plaintext.is_empty() {
                continue;
            }

            return Ok(Some(plaintext));
        }
    }
}

/// Owned write half of a [`SecretStream`], produced by [`SecretStream::into_split`].
///
/// Holds the encryptor and the write side of the underlying transport. Its
/// [`write`](SecretStreamWriteHalf::write) mirrors [`SecretStream::write`]
/// exactly.
pub struct SecretStreamWriteHalf<W> {
    raw: W,
    encrypt: secretstream::Push,
}

impl<W: AsyncWrite + Unpin + Send> SecretStreamWriteHalf<W> {
    /// Encrypt and send `data` as a single framed message.
    ///
    /// Identical framing/cipher semantics to [`SecretStream::write`].
    pub async fn write(&mut self, data: &[u8]) -> Result<(), SecretStreamError> {
        let encrypted = self.encrypt.next(data);
        // encrypted = [enc_tag(1)][ciphertext][mac(16)] = data.len() + ABYTES bytes
        debug_assert_eq!(encrypted.len(), data.len() + ABYTES);
        write_frame(&mut self.raw, &encrypted).await
    }

    /// Gracefully close the write half, sending a FIN to the remote peer.
    ///
    /// Identical to [`SecretStream::shutdown`].
    pub async fn shutdown(&mut self) -> Result<(), SecretStreamError> {
        self.raw.shutdown().await.map_err(SecretStreamError::Io)
    }
}
