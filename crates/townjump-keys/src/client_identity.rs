// JS: holesail/src/index.js:#initialise (client), holesail-client/index.js (constructor: keyPair, publicKey).
// Related: seed.rs (seed -> pair), z32.rs (`decode` yields `KeysError`).
use crate::error::KeysError;
use crate::seed::{KeyPair, key_pair_from_seed, sha256_seed};
use crate::z32;

/// Client identity: private mode - a derived key pair, public mode - only the server key from the string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientIdentity {
    /// `keyPair = HyperDHT.keyPair(z32.decode(seed_z32))`, where `seed_z32 = z32(sha256(K))`.
    Private { key_pair: KeyPair, seed_z32: String },
    /// `publicKey = z32.decode(key)`; the length is not checked (`abc` gives 2 bytes).
    Public { public_key: Vec<u8> },
}

/// `key` is the secret `K` (private mode) or the server z32 key (public mode); the error comes from `z32.decode`.
pub fn client_identity(key: &str, secure: bool) -> Result<ClientIdentity, KeysError> {
    if !secure {
        return z32::decode(key).map(|public_key| ClientIdentity::Public { public_key });
    }
    let seed = sha256_seed(key);
    Ok(ClientIdentity::Private {
        key_pair: key_pair_from_seed(&seed),
        seed_z32: z32::encode(&seed),
    })
}

#[cfg(test)]
#[path = "client_identity_tests.rs"]
mod tests;
