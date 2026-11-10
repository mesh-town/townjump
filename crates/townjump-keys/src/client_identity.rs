use crate::error::KeysError;
use crate::seed::{KeyPair, key_pair_from_seed, sha256_seed};
use crate::z32;

/// What a client presents to the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientIdentity {
    Private { key_pair: KeyPair, seed_z32: String },
    Public { public_key: Vec<u8> },
}

/// Private mode hashes the key into a pair (never fails); public mode decodes
/// the key as z32 and does not check the decoded length.
pub fn client_identity(key: &str, secure: bool) -> Result<ClientIdentity, KeysError> {
    if secure {
        let seed = sha256_seed(key);
        Ok(ClientIdentity::Private {
            key_pair: key_pair_from_seed(&seed),
            seed_z32: z32::encode(&seed),
        })
    } else {
        Ok(ClientIdentity::Public {
            public_key: z32::decode(key)?,
        })
    }
}

#[cfg(test)]
#[path = "client_identity_tests.rs"]
mod tests;
