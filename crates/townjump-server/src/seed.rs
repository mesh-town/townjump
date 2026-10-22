// JS: holesail-server/index.js:generateKeyPair (`Buffer.from(seed, 'hex')`, `HyperDHT.keyPair`); related: start.rs.
// Seed from a hex string or 32 random OS bytes; `if (!seed)` - an empty string also gives a random one.
use townjump_keys::KeysError;

/// Seed and its hex: supplied by the caller (as is) or generated.
pub(crate) fn resolve(seed_hex: Option<&str>) -> Result<([u8; 32], String), KeysError> {
    match seed_hex.filter(|s| !s.is_empty()) {
        Some(text) => Ok((decode(text)?, text.to_string())),
        None => Ok(random()),
    }
}

fn random() -> ([u8; 32], String) {
    let mut seed = [0u8; 32];
    getrandom::fill(&mut seed).expect("OS random source unavailable");
    (seed, encode(&seed))
}

/// `Buffer.from(hex, 'hex')`: parsing stops at the first invalid pair; a length other than 32 is a libsodium error.
fn decode(text: &str) -> Result<[u8; 32], KeysError> {
    let bytes: Vec<u8> = text
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map_while(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok())
        .collect();
    bytes.try_into().map_err(|_| KeysError::SeedBytes)
}

fn encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
#[path = "seed_tests.rs"]
mod tests;
