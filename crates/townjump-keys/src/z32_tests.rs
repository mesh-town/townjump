// Tests for `z32::{encode, decode}`: alphabet `ybndrfg8ejkmcpqxot1uwisza345h769` from z32/index.js; expectations are node 24.4, key-derivation.json, keyderivation-vectors.json.
use super::*;
use crate::kd_vectors::{cli_rows, field, records, unhex};

mod encode {
    use super::*;

    #[test]
    fn whole_five_byte_groups_give_eight_chars_each() {
        // node 24.4, require('z32').encode(Buffer.from(bytes))
        let cases: [(&[u8], &str); 5] = [
            (&[], ""),
            (&[0; 5], "yyyyyyyy"),
            (b"hello", "pb1sa5dx"),
            (&[0xde, 0xad, 0xbe, 0xef, 0x01], "54s575ab"),
            (&[1, 2, 3, 4, 5, 6], "yrbygbyfya"),
        ];
        for (bytes, want) in cases {
            assert_eq!(encode(bytes), want, "bytes {bytes:?}");
        }
    }

    #[test]
    fn tail_bits_are_padded_with_zeros_without_equals_signs() {
        // node 24.4: a tail of 1-4 bytes is padded with zero bits to a whole number of characters
        let cases: [(&[u8], &str); 5] = [
            (&[0], "yy"),
            (&[0xff], "9h"),
            (&[0xde], "5a"),
            (&[0xde, 0xad], "54so"),
            (&[0xde, 0xad, 0xbe, 0xef], "54s575a"),
        ];
        for (bytes, want) in cases {
            assert_eq!(encode(bytes), want, "bytes {bytes:?}");
        }
    }

    #[test]
    fn server_keys_from_vector_are_encoded_key_material() {
        for (label, rec) in records("server") {
            let source = key_material(&rec);
            assert_eq!(
                encode(&rec.res_bytes(source)),
                rec.res_str("serverKey"),
                "{label}"
            );
        }
    }

    #[test]
    fn cli_vector_seeds_and_public_keys_give_their_z32() {
        for row in cli_rows("private") {
            let seed = unhex(field(&row, "sha256Seed"));
            assert_eq!(encode(&seed), field(&row, "clientSeedZ32"), "{row}");
            let public = unhex(field(&row, "publicKey"));
            assert_eq!(encode(&public), field(&row, "publicKeyZ32"), "{row}");
        }
        let row = &cli_rows("publicNoKey")[0];
        let public = unhex(field(row, "publicKey"));
        assert_eq!(encode(&public), field(row, "publicKeyZ32"));
    }
}

mod decode {
    use super::*;

    #[test]
    fn known_text_gives_known_bytes() {
        assert_eq!(decode(""), Ok(vec![]));
        assert_eq!(decode("yy"), Ok(vec![0]));
        assert_eq!(decode("9h"), Ok(vec![0xff]));
        assert_eq!(decode("pb1sa5dx"), Ok(b"hello".to_vec()));
    }

    #[test]
    fn server_keys_from_vector_decode_to_key_material() {
        for (label, rec) in records("server") {
            let source = key_material(&rec);
            assert_eq!(
                decode(rec.res_str("serverKey")),
                Ok(rec.res_bytes(source)),
                "{label}"
            );
        }
    }

    #[test]
    fn client_public_keys_from_vector_decode_to_the_server_key() {
        let public_mode = records("client")
            .into_iter()
            .filter(|(_, rec)| !rec.secure() && rec.error.is_none());
        for (label, rec) in public_mode {
            assert_eq!(
                decode(&rec.hs_key()),
                Ok(rec.res_bytes("publicKey")),
                "{label}"
            );
        }
    }

    #[test]
    fn cli_vector_z32_seeds_decode_to_sha256_seeds() {
        for row in cli_rows("private") {
            let want = unhex(field(&row, "sha256Seed"));
            assert_eq!(decode(field(&row, "clientSeedZ32")), Ok(want), "{row}");
        }
    }

    #[test]
    fn dangling_chars_give_javascript_byte_counts() {
        // node 24.4: a tail of 1, 3 and 6 characters gives one byte more than whole bytes in its bits;
        // the missing bits are read as zeros, no error
        let cases = [
            ("y", "00"),
            ("b", "08"),
            ("yb", "00"),
            ("ybn", "0044"),
            ("ybnd", "0044"),
            ("ybndr", "004432"),
            ("ybndrf", "00443214"),
            ("ybndrfg", "00443214"),
            ("ybndrfg8", "00443214c7"),
            ("ybndrfg8e", "00443214c740"),
            ("ybndrfg8ej", "00443214c742"),
            ("ybndrfg8ejk", "00443214c74254"),
            ("ybndrfg8ejkm", "00443214c74254"),
            ("ybndrfg8ejkmc", "00443214c74254b6"),
        ];
        for (text, want) in cases {
            assert_eq!(decode(text), Ok(unhex(want)), "decode({text:?})");
        }
    }
}

mod invalid_character {
    use super::*;

    #[test]
    fn ascii_between_1_and_z_but_not_in_the_alphabet() {
        // node 24.4: `l`, `2` and capital letters are in the table but have no entry in it
        let cases = [("l", "l", 0), ("2y", "2", 0), ("yl", "l", 1), ("Y", "Y", 0)];
        for (text, ch, position) in cases {
            assert_eq!(decode(text), Err(invalid(ch, position)), "decode({text:?})");
        }
    }

    #[test]
    fn characters_outside_the_table_range() {
        // node 24.4: a code below `1` (0x31) or above `z` (0x7a)
        let cases = [
            ("ab0", "0", 2),
            ("y-", "-", 1),
            ("y y", " ", 1),
            ("y{", "{", 1),
            ("y~", "~", 1),
            ("ϑβ", "ϑ", 0),
        ];
        for (text, ch, position) in cases {
            assert_eq!(decode(text), Err(invalid(ch, position)), "decode({text:?})");
        }
    }

    #[test]
    fn the_leftmost_bad_character_is_reported() {
        assert_eq!(decode("yYyl"), Err(invalid("Y", 1)));
        assert_eq!(decode("ay1v"), Err(invalid("v", 3)));
    }

    #[test]
    fn position_counts_from_the_start_of_the_string_across_groups() {
        // node 24.4: the string is read in groups of 8 characters, the index counts from the start of the string
        assert_eq!(decode("yyyyyyyyl"), Err(invalid("l", 8)));
        assert_eq!(decode("yyyyyyyyyyyyyyyyv"), Err(invalid("v", 16)));
    }
}

mod round_trip {
    use super::*;

    #[test]
    fn every_length_up_to_64_bytes() {
        for len in 0..=64u8 {
            let bytes: Vec<u8> = (0..len)
                .map(|i| i.wrapping_mul(37).wrapping_add(len))
                .collect();
            assert_eq!(decode(&encode(&bytes)), Ok(bytes.clone()), "length {len}");
        }
    }

    #[test]
    fn vector_public_keys_round_trip() {
        for (label, rec) in records("server") {
            let bytes = unhex(rec.res_str("publicKey"));
            assert_eq!(decode(&encode(&bytes)), Ok(bytes), "{label}");
        }
    }
}

/// What `HolesailServer.key` encodes to z32: the seed in private mode (`secure === true`), otherwise the public key.
fn key_material(rec: &crate::kd_vectors::Rec) -> &'static str {
    if rec.strict_secure() {
        "generatedSeedHex"
    } else {
        "publicKey"
    }
}

fn invalid(ch: &str, position: usize) -> KeysError {
    KeysError::InvalidCharacter {
        ch: ch.to_string(),
        position,
    }
}
