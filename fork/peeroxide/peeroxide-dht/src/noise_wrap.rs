//! NoiseWrap — IK-pattern handshake with typed payload encoding.
//!
//! Combines [`HandshakeIK`](crate::noise::HandshakeIK) with [`NoisePayload`](crate::hyperdht_messages::NoisePayload) encoding/decoding,
//! and derives the `holepunch_secret` after finalisation.
//!
//! Reference: `hyperdht/lib/noise-wrap.js`.

use blake2::Blake2bMac;
use blake2::digest::consts::U32;
use blake2::digest::{KeyInit, Mac};

use crate::crypto::NS_PEER_HANDSHAKE;
use crate::crypto::NS_PEER_HOLEPUNCH;
use crate::hyperdht_messages::{
    NoisePayload, decode_noise_payload_from_bytes, encode_noise_payload_to_bytes,
};
use crate::noise::{HandshakeIK, Keypair};

type Blake2bMac256 = Blake2bMac<U32>;

// ─── Error type ──────────────────────────────────────────────────────────────

/// Errors from the [`NoiseWrap`] handshake layer.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum NoiseWrapError {
    /// The underlying Noise IK handshake failed.
    #[error("noise handshake failed: {0}")]
    Noise(#[from] crate::noise::NoiseError),

    /// Payload compact-encoding or decoding failed.
    #[error("payload encoding failed: {0}")]
    Encoding(#[from] crate::compact_encoding::EncodingError),

    /// [`NoiseWrap::finalize`] was called before both messages were exchanged.
    #[error("handshake not yet complete")]
    NotComplete,
}

// ─── Result types ────────────────────────────────────────────────────────────

/// Final output after a completed NoiseWrap handshake.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct NoiseWrapResult {
    /// Whether this side initiated the handshake.
    pub is_initiator: bool,
    /// Remote peer's static Ed25519 public key.
    pub remote_public_key: [u8; 32],
    /// Holepunch secret: `BLAKE2b-256(key=handshake_hash, data=NS_PEER_HOLEPUNCH)`.
    pub holepunch_secret: [u8; 32],
    /// Full handshake transcript hash (64 bytes).
    pub handshake_hash: [u8; 64],
    /// Session key for outbound encrypted messages.
    pub tx: [u8; 32],
    /// Session key for inbound encrypted messages.
    pub rx: [u8; 32],
}

// ─── NoiseWrap ───────────────────────────────────────────────────────────────

/// Wraps a Noise IK handshake with [`NoisePayload`] encode/decode and
/// `holepunch_secret` derivation.
///
/// # Usage
///
/// ## Initiator
/// ```ignore
/// let mut nw = NoiseWrap::new_initiator(keypair, remote_pub);
/// let m1 = nw.send(&payload)?;
/// // → send m1 to remote via PEER_HANDSHAKE relay
/// let remote_payload = nw.recv(&m2_bytes)?;
/// let result = nw.finalize()?;
/// ```
///
/// ## Responder
/// ```ignore
/// let mut nw = NoiseWrap::new_responder(keypair);
/// let remote_payload = nw.recv(&m1_bytes)?;
/// let m2 = nw.send(&payload)?;
/// let result = nw.finalize()?;
/// ```
pub struct NoiseWrap {
    handshake: HandshakeIK,
    is_initiator: bool,
}

impl NoiseWrap {
    /// Create an initiator wrapping a Noise IK handshake.
    ///
    /// `remote_public_key` is the responder's static key (obtained via findPeer).
    pub fn new_initiator(keypair: Keypair, remote_public_key: [u8; 32]) -> Self {
        let handshake = HandshakeIK::new_initiator(keypair, remote_public_key, &*NS_PEER_HANDSHAKE);
        NoiseWrap {
            handshake,
            is_initiator: true,
        }
    }

    /// Create a responder wrapping a Noise IK handshake.
    pub fn new_responder(keypair: Keypair) -> Self {
        let handshake = HandshakeIK::new_responder(keypair, &*NS_PEER_HANDSHAKE);
        NoiseWrap {
            handshake,
            is_initiator: false,
        }
    }

    /// Encode a [`NoisePayload`] and send it as the next Noise IK message.
    ///
    /// Returns the raw handshake bytes to transmit.
    pub fn send(&mut self, payload: &NoisePayload) -> Result<Vec<u8>, NoiseWrapError> {
        let encoded = encode_noise_payload_to_bytes(payload)?;
        let message = self.handshake.send(&encoded)?;
        Ok(message)
    }

    /// Receive a raw Noise IK message and decode the embedded [`NoisePayload`].
    pub fn recv(&mut self, buf: &[u8]) -> Result<NoisePayload, NoiseWrapError> {
        let plaintext = self.handshake.recv(buf)?;
        let payload = decode_noise_payload_from_bytes(&plaintext)?;
        Ok(payload)
    }

    /// Finalise the handshake and derive the holepunch secret.
    ///
    /// Must be called after both `send` and `recv` have completed (in either
    /// order depending on role).
    ///
    /// The holepunch secret is derived as:
    /// `BLAKE2b-256(key = handshake_hash[64], data = NS_PEER_HOLEPUNCH[32])`
    ///
    /// This matches Node.js `noise-wrap.js final()`:
    /// ```js
    /// crypto_generichash(out, NS.PEER_HOLEPUNCH, this.handshake.hash)
    /// ```
    /// where `crypto_generichash` with a key ≡ keyed BLAKE2b.
    pub fn finalize(self) -> Result<NoiseWrapResult, NoiseWrapError> {
        let hr = self.handshake.result().ok_or(NoiseWrapError::NotComplete)?;
        let holepunch_secret = derive_holepunch_secret(&hr.handshake_hash);

        Ok(NoiseWrapResult {
            is_initiator: self.is_initiator,
            remote_public_key: hr.remote_public_key,
            holepunch_secret,
            handshake_hash: hr.handshake_hash,
            tx: hr.tx,
            rx: hr.rx,
        })
    }

    /// Whether the handshake is complete (both messages exchanged).
    pub fn complete(&self) -> bool {
        self.handshake.complete()
    }

    /// Remote static public key: the client's key on a responder after
    /// [`recv`](Self::recv), the server's key on an initiator. `None` while unknown
    /// (a responder before `recv`).
    pub fn remote_public_key(&self) -> Option<[u8; 32]> {
        self.handshake.remote_static_key().copied()
    }

    /// Pre-set the ephemeral keypair (for deterministic testing only).
    pub fn set_ephemeral(&mut self, keypair: Keypair) {
        self.handshake.set_ephemeral(keypair);
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Derive the holepunch secret from the handshake hash.
///
/// `BLAKE2b-256(key = handshake_hash[64 bytes], data = NS_PEER_HOLEPUNCH[32 bytes])`
///
/// Matches libsodium `crypto_generichash(out, NS.PEER_HOLEPUNCH, hash)` — when
/// a key is supplied, libsodium uses BLAKE2b in keyed MAC mode.
fn derive_holepunch_secret(handshake_hash: &[u8; 64]) -> [u8; 32] {
    let mut mac: Blake2bMac256 =
        // SAFETY: BLAKE2b accepts keys from 1..=64 bytes; handshake_hash is always 64 bytes.
        KeyInit::new_from_slice(handshake_hash).expect("64-byte key valid for BLAKE2b");
    Mac::update(&mut mac, &*NS_PEER_HOLEPUNCH);
    let output = mac.finalize().into_bytes();
    let mut result = [0u8; 32];
    result.copy_from_slice(&output);
    result
}

#[cfg(test)]
#[path = "noise_wrap_tests.rs"]
mod tests;
