// JS: holesail/src/index.js:#initialise (server), holesail-server/index.js:generateKeyPair, `key`, Holesail#info.
// Related: seed.rs (seed -> pair), url.rs (`format_url`), z32.rs (public key).
use crate::seed::{KeyPair, key_pair_from_seed, sha256_seed};
use crate::url::format_url;
use crate::z32;

/// Everything `new Holesail({ server })` yields before the network. Fields are named after the columns of `testvectors/holesail/key-derivation.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerKeys {
    /// `h.key`: the secret `K`; an empty string if there is no key (public mode without `--key`).
    pub key: String,
    /// `h.seed`: `hex(sha256(K))`; `None` is public mode without a key.
    pub seed_hex: Option<String>,
    /// Seed the pair was built with (`generatedSeedHex`): `seed_hex` or random bytes as is.
    pub generated_seed_hex: String,
    pub key_pair: KeyPair,
    /// `HolesailServer.key`: `z32(seed)` in private mode, otherwise `z32(public_key)`.
    pub server_key: String,
    /// `Holesail.info.key`: `K` in private mode, otherwise `z32(public_key)`.
    pub info_key: String,
    /// `Holesail.info.url`.
    pub url: String,
}

/// Server `#initialise`. `key` is already after `urlParser` (an empty string equals `None`), `secure` is the final mode.
/// `random` fills 32 bytes (`libKeys.randomBytes`): needed by private mode without a key and public mode without a key.
pub fn server_keys(
    key: Option<&str>,
    secure: bool,
    random: &mut dyn FnMut(&mut [u8]),
) -> ServerKeys {
    let key = key.filter(|k| !k.is_empty());
    let Some(key) = key
        .map(str::to_string)
        .or_else(|| secure.then(|| random_hex(random)))
    else {
        return public_without_key(random);
    };
    let seed = sha256_seed(&key);
    from_seed(key, Some(hex::encode(seed)), seed, secure)
}

fn public_without_key(random: &mut dyn FnMut(&mut [u8])) -> ServerKeys {
    let mut seed = [0u8; 32];
    random(&mut seed);
    from_seed(String::new(), None, seed, false)
}

fn random_hex(random: &mut dyn FnMut(&mut [u8])) -> String {
    let mut bytes = [0u8; 32];
    random(&mut bytes);
    hex::encode(bytes)
}

fn from_seed(key: String, seed_hex: Option<String>, seed: [u8; 32], secure: bool) -> ServerKeys {
    let key_pair = key_pair_from_seed(&seed);
    let public_z32 = z32::encode(&key_pair.public_key);
    let (server_key, info_key) = if secure {
        (z32::encode(&seed), key.clone())
    } else {
        (public_z32.clone(), public_z32)
    };
    ServerKeys {
        key,
        seed_hex,
        generated_seed_hex: hex::encode(seed),
        key_pair,
        url: format_url(&info_key, secure),
        server_key,
        info_key,
    }
}

#[cfg(test)]
#[path = "server_keys_tests.rs"]
mod tests;
