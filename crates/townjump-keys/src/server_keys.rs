use crate::seed::{KeyPair, key_pair_from_seed, sha256_seed};
use crate::url::format_url;
use crate::z32;

/// Everything a server derives before touching the network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerKeys {
    pub key: String,
    pub seed_hex: Option<String>,
    pub generated_seed_hex: String,
    pub key_pair: KeyPair,
    pub server_key: String,
    pub info_key: String,
    pub url: String,
}

/// Derives the server's keys. `random` is called once (32 bytes) only when no
/// key text is given; an empty text counts as no key. In the public mode
/// without a key the random bytes are the seed itself, unhashed.
pub fn server_keys(
    key: Option<&str>,
    secure: bool,
    random: &mut dyn FnMut(&mut [u8]),
) -> ServerKeys {
    let given = key.filter(|k| !k.is_empty());
    let (key, seed) = match given {
        Some(k) => (k.to_string(), Some(sha256_seed(k))),
        None if secure => {
            let k = hex::encode(fresh(random));
            let s = sha256_seed(&k);
            (k, Some(s))
        }
        None => (String::new(), None),
    };
    let seed_hex = seed.map(hex::encode);
    let used = seed.unwrap_or_else(|| fresh(random));
    let key_pair = key_pair_from_seed(&used);
    let public_z32 = z32::encode(&key_pair.public_key);
    let (server_key, info_key) = if secure {
        (z32::encode(&used), key.clone())
    } else {
        (public_z32.clone(), public_z32)
    };
    let url = format_url(&info_key, secure);
    ServerKeys {
        key,
        seed_hex,
        generated_seed_hex: hex::encode(used),
        key_pair,
        server_key,
        info_key,
        url,
    }
}

fn fresh(random: &mut dyn FnMut(&mut [u8])) -> [u8; 32] {
    let mut buf = [0u8; 32];
    random(&mut buf);
    buf
}

#[cfg(test)]
#[path = "server_keys_tests.rs"]
mod tests;
