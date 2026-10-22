// JS: z32, hyper-cmd-lib-keys, holesail/src/index.js (urlParser, #initialise), holesail-client/index.js (constructor).
// Keys and the connection string without the network and without the fork; the crate depends on no other workspace crate.
pub mod client_identity;
pub mod error;
pub mod fingerprint;
pub mod seed;
pub mod server_keys;
pub mod url;
pub mod z32;

pub use client_identity::{ClientIdentity, client_identity};
pub use error::KeysError;
pub use fingerprint::fingerprint;
pub use seed::{KeyPair, key_pair_from_seed, sha256_seed};
pub use server_keys::{ServerKeys, server_keys};
pub use url::{ParsedUrl, format_url, parse_url};

#[cfg(test)]
#[path = "kd_vectors_tests.rs"]
pub(crate) mod kd_vectors;
