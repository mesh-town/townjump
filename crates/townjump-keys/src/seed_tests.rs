use super::*;

const ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
const ABC_PUB: &str = "0bdf51005e5a525cc9bfb59f18070af6a8a6013106530eb8944554a5ae4c3cfa";

#[test]
fn hash_of_abc_and_empty() {
    assert_eq!(hex::encode(sha256_seed("abc")), ABC);
    assert_eq!(
        hex::encode(sha256_seed("")),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn hash_is_over_utf8_bytes_untrimmed() {
    assert_ne!(sha256_seed(" abc"), sha256_seed("abc"));
    assert_eq!(sha256_seed("κ"), sha256_seed("\u{3ba}"));
}

#[test]
fn pair_layout_is_seed_then_public() {
    let seed: [u8; 32] = hex::decode(ABC).unwrap().try_into().unwrap();
    let kp = key_pair_from_seed(&seed);
    assert_eq!(hex::encode(kp.public_key), ABC_PUB);
    assert_eq!(&kp.secret_key[..32], &seed);
    assert_eq!(&kp.secret_key[32..], &kp.public_key);
    assert_eq!(kp.clone(), kp);
}
