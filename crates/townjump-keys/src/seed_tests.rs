// Tests for `sha256_seed` and `key_pair_from_seed`: RFC 8032 (tests 1-2), NIST sha256 vectors, key-derivation.json and keyderivation-vectors.json.
use super::*;
use crate::kd_vectors::{cli_rows, field, records, unhex, unhex32};
use crate::z32;

mod sha256_seed {
    use super::*;

    #[test]
    fn known_texts_give_known_digests() {
        // NIST/FIPS 180-4: sha256("abc"), sha256("")
        let cases = [
            (
                "abc",
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            ),
            (
                "",
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            ),
        ];
        for (text, want) in cases {
            assert_eq!(sha256_seed(text), unhex32(want), "sha256({text:?})");
        }
    }

    #[test]
    fn secrets_from_cli_vector_hash_to_their_seed() {
        for row in cli_rows("private") {
            let (secret, want) = (field(&row, "K"), field(&row, "sha256Seed"));
            assert_eq!(sha256_seed(secret), unhex32(want), "K={secret:?}");
        }
    }

    #[test]
    fn secrets_from_key_derivation_vector_hash_to_seed_hex() {
        for (label, rec) in records("server") {
            let Some(key) = rec.key_arg() else { continue };
            assert_eq!(
                sha256_seed(&key),
                unhex32(rec.res_str("seedHex")),
                "{label}"
            );
        }
    }

    #[test]
    fn hashes_utf8_bytes_of_the_text() {
        // node 24.4, createHash('sha256').update(text).digest('hex'): Greek letters are 2 bytes each, an emoji 4
        let cases = [
            (
                "κλωξ",
                "ae4d0655f9a55fb3239f70716c8ad5e9598adeabe9d6d4737e5a8279cb0d96e0",
            ),
            (
                "😀",
                "f0443a342c5ef54783a111b51ba56c938e474c32324d90c3a60c9c8e3a37e2d9",
            ),
        ];
        for (text, want) in cases {
            assert_eq!(sha256_seed(text), unhex32(want), "sha256({text:?})");
        }
    }
}

mod key_pair_from_seed {
    use super::*;

    #[test]
    fn rfc8032_vectors() {
        let cases = [
            (
                "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60",
                "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a",
            ),
            (
                "4ccd089b28ff96da9db6c346ec114e0f5b8a319f35aba624da8cf6ed4fb8a6fb",
                "3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c",
            ),
        ];
        for (seed, public) in cases {
            let pair = key_pair_from_seed(&unhex32(seed));
            assert_eq!(pair.public_key.to_vec(), unhex(public), "seed {seed}");
            assert_eq!(
                &pair.secret_key[..32],
                &unhex(seed)[..],
                "the secret key starts with the seed"
            );
            assert_eq!(
                &pair.secret_key[32..],
                &unhex(public)[..],
                "the secret key ends with the public key"
            );
        }
    }

    #[test]
    fn key_derivation_vector_pairs() {
        for (label, rec) in records("server") {
            let pair = key_pair_from_seed(&unhex32(rec.res_str("generatedSeedHex")));
            assert_eq!(
                pair.public_key.to_vec(),
                rec.res_bytes("publicKey"),
                "{label}"
            );
            assert_eq!(
                pair.secret_key.to_vec(),
                rec.res_bytes("secretKey"),
                "{label}"
            );
        }
    }

    #[test]
    fn empty_string_seed_pair() {
        // node 24.4: Ed25519 from the seed sha256(''), public key from the SPKI createPublicKey
        let seed = unhex32("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        let pair = key_pair_from_seed(&seed);
        assert_eq!(
            pair.public_key.to_vec(),
            unhex("be907b4bac84fee5ce8811db2defc9bf0b2a2a2bbc3d54d8a2257ecd70441962")
        );
    }

    #[test]
    fn cli_vector_private_rows() {
        for row in cli_rows("private") {
            check_row(&row, "sha256Seed");
        }
    }

    #[test]
    fn cli_vector_public_without_key_row() {
        for row in cli_rows("publicNoKey") {
            check_row(&row, "seedHex");
        }
    }

    fn check_row(row: &serde_json::Value, seed_field: &str) {
        let pair = key_pair_from_seed(&unhex32(field(row, seed_field)));
        let public = unhex(field(row, "publicKey"));
        assert_eq!(pair.public_key.to_vec(), public, "{row}");
        assert_eq!(
            z32::encode(&pair.public_key),
            field(row, "publicKeyZ32"),
            "{row}"
        );
    }
}
