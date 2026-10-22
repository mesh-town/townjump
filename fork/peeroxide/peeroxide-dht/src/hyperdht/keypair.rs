//! `KeyPair`: an ed25519 pair with seed constructor, Noise conversion and a redacting `Debug`.

use std::fmt;

use ed25519_dalek::SigningKey;
use rand::random;

use crate::noise::Keypair as NoiseKeypair;

// ── KeyPair ───────────────────────────────────────────────────────────────────

#[derive(Clone)]
/// An Ed25519 key pair (libsodium layout: seed‖public_key).
pub struct KeyPair {
    /// The 32-byte public key.
    pub public_key: [u8; 32],
    /// The 64-byte secret key in libsodium layout.
    pub secret_key: [u8; 64],
}

impl KeyPair {
    /// Generate a new random key pair.
    pub fn generate() -> Self {
        let seed: [u8; 32] = random();
        Self::from_seed(seed)
    }

    /// Derive a deterministic key pair from a 32-byte seed.
    pub fn from_seed(seed: [u8; 32]) -> Self {
        let signing_key = SigningKey::from_bytes(&seed);
        let pk: [u8; 32] = signing_key.verifying_key().to_bytes();
        let mut sk = [0u8; 64];
        sk[..32].copy_from_slice(&seed);
        sk[32..].copy_from_slice(&pk);
        Self {
            public_key: pk,
            secret_key: sk,
        }
    }
}

impl fmt::Debug for KeyPair {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyPair")
            .field("public_key", &to_hex(self.public_key))
            .finish_non_exhaustive()
    }
}

impl KeyPair {
    pub(crate) fn to_noise_keypair(&self) -> NoiseKeypair {
        NoiseKeypair {
            public_key: self.public_key,
            secret_key: self.secret_key,
        }
    }
}

fn to_hex(bytes: impl AsRef<[u8]>) -> String {
    let bytes = bytes.as_ref();
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            use std::fmt::Write;
            write!(s, "{b:02x}").ok();
            s
        })
}
