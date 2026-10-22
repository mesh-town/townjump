// JS: `crypto.createHash('sha256')` in holesail, `HyperDHT.keyPair(seed)` (crypto_sign_seed_keypair).
// Related: server_keys.rs and client_identity.rs build pairs from the seed.
use ed25519_dalek::SigningKey;
use sha2::{Digest, Sha256};

/// Ed25519 pair in the libsodium layout: secret = seed ‖ public key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyPair {
    pub public_key: [u8; 32],
    pub secret_key: [u8; 64],
}

/// Pair from a 32-byte seed.
pub fn key_pair_from_seed(seed: &[u8; 32]) -> KeyPair {
    let public_key = SigningKey::from_bytes(seed).verifying_key().to_bytes();
    let mut secret_key = [0u8; 64];
    secret_key[..32].copy_from_slice(seed);
    secret_key[32..].copy_from_slice(&public_key);
    KeyPair {
        public_key,
        secret_key,
    }
}

/// SHA-256 of the UTF-8 bytes of a string.
pub fn sha256_seed(key: &str) -> [u8; 32] {
    Sha256::digest(key.as_bytes()).into()
}

#[cfg(test)]
#[path = "seed_tests.rs"]
mod tests;
