// JS: exceptions that `HolesailServer` lets through (DHT errors and `HyperDHT.keyPair`).
// `Display` is the JS exception `.message`.
use townjump_dht::DhtError;
use townjump_keys::KeysError;

/// Server errors.
#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error(transparent)]
    Dht(#[from] DhtError),
    /// Seed of the wrong length (`seed must be 'crypto_sign_SEEDBYTES' bytes`).
    #[error(transparent)]
    Keys(#[from] KeysError),
    /// `put`/`get` before `start`: in JS there is no key pair yet.
    #[error("server is not started")]
    NotStarted,
}

#[cfg(test)]
#[path = "error_tests.rs"]
mod tests;
