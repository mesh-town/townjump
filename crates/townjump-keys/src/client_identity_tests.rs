// Tests for `client_identity`: client records of key-derivation.json (private and public mode, 10 z32 errors), keyderivation-vectors.json.
use super::*;
use crate::error::KeysError;
use crate::kd_vectors::{Rec, cli_rows, field, records, unhex};
use crate::server_keys::server_keys;

mod private_mode {
    use super::*;

    #[test]
    fn gives_key_pair_and_seed_z32_from_vector() {
        for (label, rec) in modes(true) {
            let (key_pair, seed_z32) = private(&rec.hs_key());
            assert_eq!(seed_z32, rec.res_str("seedZ32OrKey"), "{label}: seed_z32");
            let want = &rec.res("keyPair");
            let public = unhex(want["publicKey"].as_str().expect("publicKey"));
            let secret = unhex(want["secretKey"].as_str().expect("secretKey"));
            assert_eq!(key_pair.public_key.to_vec(), public, "{label}: public_key");
            assert_eq!(key_pair.secret_key.to_vec(), secret, "{label}: secret_key");
        }
    }

    #[test]
    fn cli_vector_rows_give_seed_z32_and_pair() {
        for row in cli_rows("private") {
            let (key_pair, seed_z32) = private(field(&row, "K"));
            assert_eq!(seed_z32, field(&row, "clientSeedZ32"), "{row}");
            let public = unhex(field(&row, "publicKey"));
            assert_eq!(key_pair.public_key.to_vec(), public, "{row}");
            let secret = [unhex(field(&row, "sha256Seed")), public].concat();
            assert_eq!(
                key_pair.secret_key.to_vec(),
                secret,
                "{row}: seed ‖ public key"
            );
        }
    }

    #[test]
    fn empty_key_is_hashed_like_any_other() {
        // node 24.4: z32.encode(sha256('')), Ed25519 from this seed
        let (key_pair, seed_z32) = private("");
        assert_eq!(
            seed_z32,
            "hqaceowa9oqbjgz56urj1573rou4hoxrc1p3gufr1scts6n1zbko"
        );
        let public = "be907b4bac84fee5ce8811db2defc9bf0b2a2a2bbc3d54d8a2257ecd70441962";
        assert_eq!(key_pair.public_key.to_vec(), unhex(public));
    }

    #[test]
    fn pair_is_the_one_the_server_builds_from_the_same_secret() {
        // the server tells clients apart by public key (firewall): for both it is `0bdf51…`
        let public = unhex("0bdf51005e5a525cc9bfb59f18070af6a8a6013106530eb8944554a5ae4c3cfa");
        let (key_pair, _) = private("abc");
        let server = server_keys(Some("abc"), true, &mut |_: &mut [u8]| {});
        assert_eq!(key_pair.public_key.to_vec(), public);
        assert_eq!(key_pair, server.key_pair);
    }
}

mod public_mode {
    use super::*;

    #[test]
    fn gives_decoded_server_key_from_vector() {
        for (label, rec) in modes(false)
            .into_iter()
            .filter(|(_, rec)| rec.error.is_none())
        {
            assert_eq!(public(&rec.hs_key()), rec.res_bytes("publicKey"), "{label}");
        }
    }

    #[test]
    fn invalid_key_fails_with_javascript_message() {
        let errors: Vec<_> = modes(false)
            .into_iter()
            .filter(|(_, rec)| rec.error.is_some())
            .collect();
        assert_eq!(errors.len(), 10, "the vector has ten client errors");
        for (label, rec) in errors {
            let got = client_identity(&rec.hs_key(), false).expect_err(&label);
            assert!(
                matches!(got, KeysError::InvalidCharacter { .. }),
                "{label}: {got:?}"
            );
            let message = rec.error_message().expect("message");
            assert_eq!(got.to_string(), message, "{label}");
        }
    }

    #[test]
    fn length_of_the_key_is_not_checked() {
        // key-derivation.json, record 5: `abc` gives two bytes `c0 58`, no error
        assert_eq!(public("abc"), vec![0xc0, 0x58]);
    }

    #[test]
    fn empty_key_gives_empty_public_key() {
        // node 24.4: z32.decode('').length === 0
        assert_eq!(public(""), Vec::<u8>::new());
    }
}

mod reversed_public_flag {
    use super::*;

    const HEX_KEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

    #[test]
    fn q1_bare_hex_key_without_the_flag_is_not_z32() {
        // without `--public` the client reads a bare 64-hex key as a public one; `0` is outside the z32 alphabet
        let want = KeysError::InvalidCharacter {
            ch: "0".to_string(),
            position: 0,
        };
        assert_eq!(client_identity(HEX_KEY, false), Err(want));
    }

    #[test]
    fn q1_same_key_with_the_flag_is_private() {
        let (key_pair, seed_z32) = private(HEX_KEY);
        assert_eq!(
            seed_z32,
            "p1dcpksf9c1m37q31qqmxi6ictqq8fya6tr6yq3gfzkxwffwzrio"
        );
        let public = "cd38df7e4940e276ebf8d3d2ceedb583f46a2beb2c66f131aba5ad35406ff4b5";
        assert_eq!(key_pair.public_key.to_vec(), unhex(public));
    }
}

/// Client records of one mode (`secure` is the JS truthiness of `holesail.secure`).
fn modes(secure: bool) -> Vec<(String, Rec)> {
    records("client")
        .into_iter()
        .filter(|(_, rec)| rec.secure() == secure)
        .collect()
}

/// Private identity: the pair and the z32 of the seed; any other result or an error fails the test.
fn private(key: &str) -> (KeyPair, String) {
    match client_identity(key, true) {
        Ok(ClientIdentity::Private { key_pair, seed_z32 }) => (key_pair, seed_z32),
        other => panic!("client_identity({key:?}, true): expected Private, got {other:?}"),
    }
}

/// Public identity: the server key bytes; any other result or an error fails the test.
fn public(key: &str) -> Vec<u8> {
    match client_identity(key, false) {
        Ok(ClientIdentity::Public { public_key }) => public_key,
        other => panic!("client_identity({key:?}, false): expected Public, got {other:?}"),
    }
}
