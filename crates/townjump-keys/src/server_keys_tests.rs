// Tests for `server_keys`: records of key-derivation.json and rows of keyderivation-vectors.json; randomness `00 01 … 1f` where no key is set.
use super::*;
use crate::kd_vectors::{Rec, cli_rows, counting_random, field, records, unhex};
use std::cell::Cell;

mod every_vector_record {
    use super::*;

    #[test]
    fn keeps_key_and_seed() {
        for (label, rec) in records("server") {
            let (keys, _) = run(&rec);
            assert_eq!(keys.key, rec.hs_key(), "{label}: key");
            assert_eq!(
                keys.seed_hex.as_deref(),
                rec.res("seedHex").as_str(),
                "{label}: seed_hex"
            );
            assert_eq!(
                keys.generated_seed_hex,
                rec.res_str("generatedSeedHex"),
                "{label}: generated_seed_hex"
            );
        }
    }

    #[test]
    fn builds_the_key_pair() {
        for (label, rec) in records("server") {
            let (keys, _) = run(&rec);
            assert_eq!(
                keys.key_pair.public_key.to_vec(),
                rec.res_bytes("publicKey"),
                "{label}: public_key"
            );
            assert_eq!(
                keys.key_pair.secret_key.to_vec(),
                rec.res_bytes("secretKey"),
                "{label}: secret_key"
            );
        }
    }

    #[test]
    fn writes_server_key_info_key_and_url() {
        for (label, rec) in records("server") {
            let (keys, _) = run(&rec);
            if !is_text_false(&rec) {
                let want = rec.res_str("serverKey");
                assert_eq!(keys.server_key, want, "{label}: server_key");
            }
            assert_eq!(keys.info_key, rec.res_str("infoKey"), "{label}: info_key");
            assert_eq!(keys.url, rec.res_str("url"), "{label}: url");
        }
    }

    #[test]
    fn asks_for_random_bytes_only_when_key_is_missing() {
        for (label, rec) in records("server") {
            let (_, calls) = run(&rec);
            let want = usize::from(rec.key_arg().is_none());
            assert_eq!(calls, want, "{label}: number of randomness requests");
        }
    }
}

mod private_mode_with_key {
    use super::*;

    #[test]
    fn seed_is_sha256_of_the_key_text() {
        let keys = derive(Some("abc"), true);
        assert_eq!(keys.key, "abc");
        assert_eq!(keys.seed_hex.as_deref(), Some(ABC_SEED_HEX));
        assert_eq!(keys.generated_seed_hex, ABC_SEED_HEX);
        assert_eq!(keys.key_pair.public_key.to_vec(), unhex(ABC_PUBLIC_HEX));
    }

    #[test]
    fn info_shows_the_secret_and_the_server_key_is_z32_of_the_seed() {
        let keys = derive(Some("abc"), true);
        assert_eq!(keys.info_key, "abc");
        assert_eq!(keys.url, "hs://s000abc");
        assert_eq!(keys.server_key, ABC_SEED_Z32);
    }
}

mod public_mode_with_key {
    use super::*;

    #[test]
    fn seed_is_still_sha256_of_the_key_text() {
        let keys = derive(Some("abc"), false);
        assert_eq!(keys.key, "abc");
        assert_eq!(keys.seed_hex.as_deref(), Some(ABC_SEED_HEX));
        assert_eq!(keys.key_pair.public_key.to_vec(), unhex(ABC_PUBLIC_HEX));
    }

    #[test]
    fn info_shows_the_public_key_and_hides_the_secret() {
        // `--key` stays in `key`, but is neither in `info.key` nor in `url`
        let keys = derive(Some("abc"), false);
        assert_eq!(keys.info_key, ABC_PUBLIC_Z32);
        assert_eq!(keys.server_key, ABC_PUBLIC_Z32);
        assert_eq!(keys.url, format!("hs://0000{ABC_PUBLIC_Z32}"));
    }
}

mod private_mode_without_key {
    use super::*;

    #[test]
    fn random_bytes_become_the_hex_key() {
        let keys = derive(None, true);
        assert_eq!(keys.key, RANDOM_HEX);
        assert_eq!(keys.info_key, RANDOM_HEX);
        assert_eq!(keys.url, format!("hs://s000{RANDOM_HEX}"));
    }

    #[test]
    fn seed_is_sha256_of_the_hex_text_not_of_the_raw_bytes() {
        let keys = derive(None, true);
        assert_eq!(keys.seed_hex.as_deref(), Some(RANDOM_TEXT_SEED_HEX));
        assert_eq!(keys.generated_seed_hex, RANDOM_TEXT_SEED_HEX);
        assert_eq!(keys.server_key, RANDOM_TEXT_SEED_Z32);
    }
}

mod public_mode_without_key {
    use super::*;

    #[test]
    fn random_bytes_are_the_seed_as_is_without_hashing() {
        let keys = derive(None, false);
        assert_eq!(keys.key, "");
        assert_eq!(keys.seed_hex, None);
        assert_eq!(keys.generated_seed_hex, RANDOM_HEX);
        assert_eq!(keys.key_pair.public_key.to_vec(), unhex(RANDOM_PUBLIC_HEX));
    }

    #[test]
    fn info_shows_the_public_key() {
        let keys = derive(None, false);
        assert_eq!(keys.server_key, RANDOM_PUBLIC_Z32);
        assert_eq!(keys.info_key, RANDOM_PUBLIC_Z32);
        assert_eq!(keys.url, format!("hs://0000{RANDOM_PUBLIC_Z32}"));
    }
}

mod empty_key {
    use super::*;

    #[test]
    fn in_public_mode_is_the_same_as_no_key() {
        assert_eq!(derive(Some(""), false), derive(None, false));
    }

    #[test]
    fn in_private_mode_is_the_same_as_no_key() {
        let keys = derive(Some(""), true);
        assert_eq!(
            keys.key, RANDOM_HEX,
            "an empty key is falsy, the key is built from randomness"
        );
        assert_eq!(keys.seed_hex.as_deref(), Some(RANDOM_TEXT_SEED_HEX));
    }
}

mod cli_vector {
    use super::*;

    #[test]
    fn private_rows() {
        for row in cli_rows("private") {
            let (secret, keys) = (field(&row, "K"), derive(Some(field(&row, "K")), true));
            assert_eq!(keys.url, field(&row, "url"), "K={secret:?}");
            assert_eq!(keys.info_key, secret, "K={secret:?}");
            assert_eq!(
                keys.server_key,
                field(&row, "clientSeedZ32"),
                "K={secret:?}"
            );
            let public = unhex(field(&row, "publicKey"));
            assert_eq!(keys.key_pair.public_key.to_vec(), public, "K={secret:?}");
        }
    }

    #[test]
    fn public_without_key_row() {
        let row = &cli_rows("publicNoKey")[0];
        let keys = derive(None, false);
        assert_eq!(keys.url, field(row, "url"));
        assert_eq!(keys.server_key, field(row, "publicKeyZ32"));
        assert_eq!(keys.generated_seed_hex, field(row, "seedHex"));
        assert_eq!(
            keys.key_pair.public_key.to_vec(),
            unhex(field(row, "publicKey"))
        );
    }
}

// Values from key-derivation.json and keyderivation-vectors.json (`abc`, randomness `00 01 … 1f`).
const ABC_SEED_HEX: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
const ABC_SEED_Z32: &str = "zjhbpxhxy886wokbedxf5mtnrqaygapd1amzi8fwnd9sdhoynsso";
const ABC_PUBLIC_HEX: &str = "0bdf51005e5a525cc9bfb59f18070af6a8a6013106530eb8944554a5ae4c3cfa";
const ABC_PUBLIC_Z32: &str = "bxxinyn6mjjf31p9ssxtobak64wkcyjty3jo7qrweikkmm1c8u7y";
const RANDOM_HEX: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
const RANDOM_TEXT_SEED_HEX: &str =
    "6c86c6aac5fb24bcf5d9939cb7d7d5645ce39418f449e03b262dd4fa14b4b92b";
const RANDOM_TEXT_SEED_Z32: &str = "p1dcpksf9c1m37q31qqmxi6ictqq8fya6tr6yq3gfzkxwffwzrio";
const RANDOM_PUBLIC_HEX: &str = "03a107bff3ce10be1d70dd18e74bc09967e4d6309ba50d5f1ddc8664125531b8";
const RANDOM_PUBLIC_Z32: &str = "yqooxx9u3aemh8mo5wcqq16yufu6jitouq1o4za751dger1igghy";

/// `server_keys` with the source `00 01 … 1f`.
fn derive(key: Option<&str>, secure: bool) -> ServerKeys {
    server_keys(key, secure, &mut counting_random(&Cell::new(0)))
}

/// Runs `server_keys` for a vector record; returns the result and the number of randomness requests.
fn run(rec: &Rec) -> (ServerKeys, usize) {
    let calls = Cell::new(0);
    let key = rec.key_arg();
    let keys = server_keys(key.as_deref(), rec.secure(), &mut counting_random(&calls));
    (keys, calls.get())
}

/// `secure: "false"` (a string): truthy for JS, while `HolesailServer.key` takes `secure === true` and gives `z32(public_key)`.
/// JS (`testvectors/probes-js/keys.cjs`): `new Holesail({ server: true, secure: 'false' })` is a hybrid: keys are derived
/// as for private mode (`holesail/src/index.js:#initialise` checks truthiness), while the server sees `secure === true` → false and
/// runs publicly. `secure: bool` cannot express this, so `server_key` of such records is not compared; the CLI never passes a string
/// (`!argv.public` is always boolean) - `spec/holesail/server.md`, «The `secure` string».
fn is_text_false(rec: &Rec) -> bool {
    rec.secure() && !rec.strict_secure()
}
