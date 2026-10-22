//! `SecretStream`: Noise XX handshake, ID header exchange and framed read/write.
// JS: @hyperswarm/secret-stream/index.js

use std::task::{Context, Poll, ready};

use tokio::io::{AsyncRead, AsyncWrite};
use tracing::debug;

use super::error::SecretStreamError;
use super::frame::{IDHEADERBYTES, read_frame, stream_id, write_frame, write_frame_unflushed};
use super::io::IoState;
use super::timers::Timers;
use crate::noise::{self, Keypair};
use crate::secretstream::{self, ABYTES, HEADERBYTES, KEYBYTES};

/// An encrypted bidirectional stream.
///
/// Wraps any `AsyncRead + AsyncWrite` transport with a Noise XX handshake
/// followed by libsodium-compatible secretstream encryption.
pub struct SecretStream<T> {
    pub(super) raw: T,
    pub(super) encrypt: secretstream::Push,
    /// `None` until the peer's ID header has been read (`from_session` reads it lazily).
    pub(super) decrypt: Option<secretstream::Pull>,
    pub(super) pending_header: Option<PendingHeader>,
    pub(super) remote_public_key: [u8; 32],
    pub(super) handshake_hash: [u8; 64],
    pub(super) is_initiator: bool,
    pub(super) io: IoState,
    pub(super) timers: Timers,
}

impl<T: AsyncRead + AsyncWrite + Unpin + Send> SecretStream<T> {
    /// Perform the Noise XX handshake + ID header exchange, returning a ready
    /// encrypted stream.
    ///
    /// - `is_initiator`: true for the connecting side, false for the accepting side.
    /// - `raw`: the underlying transport (e.g. `TcpStream`, UDX stream adapter).
    /// - `keypair`: the local Ed25519 static keypair.
    pub async fn new(
        is_initiator: bool,
        mut raw: T,
        keypair: Keypair,
    ) -> Result<Self, SecretStreamError> {
        // ── Phase 1: Noise XX handshake ──────────────────────────────────
        let mut handshake = noise::Handshake::new(is_initiator, keypair);

        if is_initiator {
            // → M1
            let m1 = handshake.send().map_err(SecretStreamError::Noise)?;
            write_frame(&mut raw, &m1).await?;

            // ← M2
            let m2 = read_frame(&mut raw)
                .await?
                .ok_or(SecretStreamError::UnexpectedEof)?;
            handshake.recv(&m2).map_err(SecretStreamError::Noise)?;

            // → M3
            let m3 = handshake.send().map_err(SecretStreamError::Noise)?;
            write_frame(&mut raw, &m3).await?;
        } else {
            // ← M1
            let m1 = read_frame(&mut raw)
                .await?
                .ok_or(SecretStreamError::UnexpectedEof)?;
            handshake.recv(&m1).map_err(SecretStreamError::Noise)?;

            // → M2
            let m2 = handshake.send().map_err(SecretStreamError::Noise)?;
            write_frame(&mut raw, &m2).await?;

            // ← M3
            let m3 = read_frame(&mut raw)
                .await?
                .ok_or(SecretStreamError::UnexpectedEof)?;
            handshake.recv(&m3).map_err(SecretStreamError::Noise)?;
        }

        let hr = handshake
            .result()
            .ok_or(SecretStreamError::HandshakeIncomplete)?
            .clone();

        debug!(is_initiator, "noise handshake complete");

        // ── Phase 2: ID header exchange ──────────────────────────────────
        // Create encryptor → produces the 24-byte secretstream header.
        let tx_key: [u8; KEYBYTES] = hr.tx;
        let (encrypt, ss_header) = secretstream::Push::new(&tx_key);

        // Build local ID message: [32B stream_id][24B secretstream_header]
        let local_id = stream_id(&hr.handshake_hash, is_initiator);
        let mut id_msg = Vec::with_capacity(IDHEADERBYTES);
        id_msg.extend_from_slice(&local_id);
        id_msg.extend_from_slice(&ss_header);
        // No flush: it would wait for the peer's ACK of this header (one RTT) before we read theirs.
        write_frame_unflushed(&mut raw, &id_msg).await?;

        // Read remote ID message.
        let remote_msg = read_frame(&mut raw)
            .await?
            .ok_or(SecretStreamError::UnexpectedEof)?;
        if remote_msg.len() != IDHEADERBYTES {
            return Err(SecretStreamError::InvalidIdHeader {
                expected: IDHEADERBYTES,
                got: remote_msg.len(),
            });
        }

        // Validate remote stream ID.
        let remote_id = &remote_msg[..32];
        let expected_id = stream_id(&hr.handshake_hash, !is_initiator);
        if remote_id != expected_id {
            return Err(SecretStreamError::StreamIdMismatch);
        }

        // Initialize decryptor with remote's secretstream header.
        let remote_header: [u8; HEADERBYTES] = remote_msg[32..56]
            .try_into()
            // SAFETY: remote_msg.len() == IDHEADERBYTES (56) verified above; [32..56] is 24 bytes == HEADERBYTES.
            .expect("slice length verified above");
        let rx_key: [u8; KEYBYTES] = hr.rx;
        let decrypt = secretstream::Pull::new(&rx_key, &remote_header);

        debug!(is_initiator, "secret stream established");

        Ok(Self {
            raw,
            encrypt,
            decrypt: Some(decrypt),
            pending_header: None,
            remote_public_key: hr.remote_public_key,
            handshake_hash: hr.handshake_hash,
            is_initiator,
            io: IoState::default(),
            timers: Timers::new(),
        })
    }

    /// Create a SecretStream from pre-negotiated session keys (e.g. from Noise IK).
    ///
    /// Skips the Noise XX handshake but still performs the ID header exchange
    /// (stream identity + secretstream header). Used for relay connections and
    /// direct connections where Noise IK already established the session.
    pub async fn from_session(
        is_initiator: bool,
        mut raw: T,
        tx: [u8; 32],
        rx: [u8; 32],
        handshake_hash: [u8; 64],
        remote_public_key: [u8; 32],
    ) -> Result<Self, SecretStreamError> {
        let (encrypt, ss_header) = secretstream::Push::new(&tx);

        let local_id = stream_id(&handshake_hash, is_initiator);
        let mut id_msg = Vec::with_capacity(IDHEADERBYTES);
        id_msg.extend_from_slice(&local_id);
        id_msg.extend_from_slice(&ss_header);
        // No flush: it would wait for the peer's ACK of this header (one RTT) before we read theirs.
        write_frame_unflushed(&mut raw, &id_msg).await?;

        // The peer's header is read at the first read, not here: like JS `es.open`, the
        // stream is usable (and its socket can be held) right after our own header is sent.
        let pending_header = PendingHeader {
            expected_id: stream_id(&handshake_hash, !is_initiator),
            rx,
        };

        debug!(is_initiator, "secret stream established (from session)");

        Ok(Self {
            raw,
            encrypt,
            decrypt: None,
            pending_header: Some(pending_header),
            remote_public_key,
            handshake_hash,
            is_initiator,
            io: IoState::default(),
            timers: Timers::new(),
        })
    }

    /// Encrypt and send `data` as a single framed message.
    pub async fn write(&mut self, data: &[u8]) -> Result<(), SecretStreamError> {
        // A frame left half-written by `AsyncWrite::poll_write` goes out first.
        std::future::poll_fn(|cx| self.poll_drain_tx(cx)).await?;
        let encrypted = self.encrypt.next(data);
        // encrypted = [enc_tag(1)][ciphertext][mac(16)] = data.len() + ABYTES bytes
        debug_assert_eq!(encrypted.len(), data.len() + ABYTES);
        self.timers.tx_sent();
        write_frame(&mut self.raw, &encrypted).await
    }

    /// Gracefully close the write half, sending a FIN to the remote peer.
    pub async fn shutdown(&mut self) -> Result<(), SecretStreamError> {
        use tokio::io::AsyncWriteExt;
        self.timers.stop_keep_alive();
        std::future::poll_fn(|cx| self.poll_drain_tx(cx)).await?;
        self.raw.shutdown().await.map_err(SecretStreamError::Io)
    }

    /// Read and decrypt the next framed message.
    ///
    /// Returns `Ok(None)` on clean EOF, `Ok(Some(plaintext))` for data messages.
    /// Empty messages (keepalives) are silently consumed and the next message
    /// is read. Cancel-safe: a frame half-read when the future is dropped is
    /// finished by the next call. Serves the keepalive and idle-timeout timers
    /// (`TimedOut` arrives as [`SecretStreamError::Io`]).
    pub async fn read(&mut self) -> Result<Option<Vec<u8>>, SecretStreamError> {
        std::future::poll_fn(|cx| self.poll_peer_header(cx)).await?;
        loop {
            let frame = std::future::poll_fn(|cx| self.poll_frame_timed(cx))
                .await
                .map_err(frame_error)?;
            let Some(frame) = frame else {
                return Ok(None);
            };
            let plaintext = self.decrypt_frame(&frame)?;
            // Drop empty keepalive messages (match Node.js behavior).
            if !plaintext.is_empty() {
                return Ok(Some(plaintext));
            }
        }
    }

    fn decrypt_frame(&mut self, frame: &[u8]) -> Result<Vec<u8>, SecretStreamError> {
        if frame.len() < ABYTES {
            return Err(SecretStreamError::Decrypt(
                secretstream::SecretstreamError::CiphertextTooShort,
            ));
        }
        let decrypt = self
            .decrypt
            .as_mut()
            .ok_or(SecretStreamError::HandshakeIncomplete)?;
        let (plaintext, _tag) = decrypt.next(frame)?;
        Ok(plaintext)
    }

    /// Remote peer's static Ed25519 public key.
    pub fn remote_public_key(&self) -> &[u8; 32] {
        &self.remote_public_key
    }

    /// BLAKE2b-512 hash of the Noise handshake transcript.
    pub fn handshake_hash(&self) -> &[u8; 64] {
        &self.handshake_hash
    }

    /// Whether this side initiated the connection.
    pub fn is_initiator(&self) -> bool {
        self.is_initiator
    }

    /// The underlying transport, e.g. to read its peer address.
    pub fn get_ref(&self) -> &T {
        &self.raw
    }

    /// Consume the wrapper, returning the underlying transport.
    pub fn into_inner(self) -> T {
        self.raw
    }
}

/// What `from_session` still owes the peer's ID header: identity to expect and the receive key.
pub(super) struct PendingHeader {
    expected_id: [u8; 32],
    rx: [u8; KEYBYTES],
}

impl PendingHeader {
    /// Check the peer's ID message and start the decryptor from its secretstream header.
    pub(super) fn accept(&self, msg: &[u8]) -> Result<secretstream::Pull, SecretStreamError> {
        if msg.len() != IDHEADERBYTES {
            return Err(SecretStreamError::InvalidIdHeader {
                expected: IDHEADERBYTES,
                got: msg.len(),
            });
        }
        if msg[..32] != self.expected_id {
            return Err(SecretStreamError::StreamIdMismatch);
        }
        let header: [u8; HEADERBYTES] = msg[32..56]
            .try_into()
            // SAFETY: msg.len() == IDHEADERBYTES (56) verified above; [32..56] is 24 bytes == HEADERBYTES.
            .expect("slice length verified above");
        Ok(secretstream::Pull::new(&self.rx, &header))
    }
}

impl<T: AsyncRead + AsyncWrite + Unpin> SecretStream<T> {
    /// Read the peer's ID header if `from_session` has not yet; resumable and cancel-safe.
    /// A bad header fails here (first read) and stays pending.
    pub(super) fn poll_peer_header(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), SecretStreamError>> {
        if self.pending_header.is_none() {
            return Poll::Ready(Ok(()));
        }
        let msg = ready!(self.poll_frame_timed(cx))
            .map_err(frame_error)?
            .ok_or(SecretStreamError::UnexpectedEof)?;
        let decrypt = self
            .pending_header
            .as_ref()
            .ok_or(SecretStreamError::HandshakeIncomplete)?
            .accept(&msg)?;
        self.decrypt = Some(decrypt);
        self.pending_header = None;
        Poll::Ready(Ok(()))
    }
}

/// EOF inside a frame keeps its handshake-style error; any other transport error stays `Io`.
fn frame_error(e: std::io::Error) -> SecretStreamError {
    if e.kind() == std::io::ErrorKind::UnexpectedEof {
        return SecretStreamError::UnexpectedEof;
    }
    SecretStreamError::Io(e)
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
