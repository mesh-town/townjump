// JS: `HyperDHT.keyPair(seed)` (libsodium crypto_sign_seed_keypair); the pairs are the same for the mock and the live network.
// Related: `townjump-keys` computes the same, but testkit computes its own so as not to depend on it.
use ed25519_dalek::SigningKey;
use townjump_keys::KeyPair;

/// Ed25519 pair from a seed: secret = seed ‖ public key (libsodium layout).
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

/// Pair by tag: seed `[tag; 32]`; different tags give different pairs.
pub fn key_pair(tag: u8) -> KeyPair {
    key_pair_from_seed(&[tag; 32])
}

#[cfg(test)]
#[path = "keys_tests.rs"]
mod tests;
