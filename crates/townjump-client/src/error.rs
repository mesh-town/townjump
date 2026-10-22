// JS: exceptions that `HolesailClient` lets through (z32, JSON.parse of a foreign record, DHT).
// `Display` is the JS exception `.message`.
use townjump_dht::DhtError;
use townjump_jsval::JsError;
use townjump_keys::KeysError;

/// Client errors.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error(transparent)]
    Dht(#[from] DhtError),
    /// Invalid z32 and a seed of the wrong length in `ping`.
    #[error(transparent)]
    Keys(#[from] KeysError),
    /// `JSON.parse` of a foreign record and access to fields of `null`.
    #[error(transparent)]
    Js(#[from] JsError),
}

#[cfg(test)]
#[path = "error_tests.rs"]
mod tests;
