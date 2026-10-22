use super::*;
use crate::vectors;

#[test]
fn key_pair_layout() {
    let pair = key_pair(0x11);
    assert_eq!(pair.secret_key[..32], [0x11; 32]);
    assert_eq!(pair.secret_key[32..], pair.public_key);
}

#[test]
fn different_tags_differ() {
    assert_ne!(key_pair(1).public_key, key_pair(2).public_key);
    assert_eq!(key_pair(1), key_pair_from_seed(&[1; 32]));
}

fn hex_array<const N: usize>(text: &str) -> [u8; N] {
    hex::decode(text).unwrap().try_into().unwrap()
}

#[test]
fn matches_js_vector() {
    let table = vectors::load_value("holesail/key-derivation.json");
    let result = table["vectors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| &entry["result"])
        .find(|r| r["generatedSeedHex"].is_string() && r["publicKey"].is_string())
        .expect("key-derivation.json has no record with a seed and a public key");
    let seed: [u8; 32] = hex_array(result["generatedSeedHex"].as_str().unwrap());
    let pair = key_pair_from_seed(&seed);
    assert_eq!(
        hex::encode(pair.public_key),
        result["publicKey"].as_str().unwrap()
    );
    assert_eq!(
        hex::encode(pair.secret_key),
        result["secretKey"].as_str().unwrap()
    );
}
