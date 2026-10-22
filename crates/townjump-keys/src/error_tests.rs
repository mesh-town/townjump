// Tests for `KeysError`: the text and fields of a `z32.decode` error are checked against `error.message` of the key-derivation.json records.
// No crate function returns `KeysError::SeedBytes` (JS: `--lookup` without a value, hyperdht layer), so there are no tests.
use super::*;
use crate::kd_vectors::records;
use crate::z32;

mod invalid_character {
    use super::*;

    #[test]
    fn vector_errors_carry_char_and_position_from_the_message() {
        let errors: Vec<_> = records("client")
            .into_iter()
            .filter(|(_, rec)| rec.error.is_some())
            .collect();
        assert_eq!(errors.len(), 10, "the vector has ten client errors");
        for (label, rec) in errors {
            let message = rec.error_message().expect("message");
            let (ch, position) = split_message(message);
            let want = KeysError::InvalidCharacter { ch, position };
            assert_eq!(z32::decode(&rec.hs_key()), Err(want), "{label}");
        }
    }

    #[test]
    fn display_is_the_message_of_the_vector() {
        for (label, rec) in records("client")
            .into_iter()
            .filter(|(_, rec)| rec.error.is_some())
        {
            let got = z32::decode(&rec.hs_key()).expect_err("error");
            assert_eq!(
                got.to_string(),
                rec.error_message().expect("message"),
                "{label}"
            );
        }
    }

    #[test]
    fn non_ascii_character_is_quoted_as_is() {
        // node 24.4: try { z32.decode('ϑβ') } catch (e) { e.message }
        let got = z32::decode("ϑβ").expect_err("not in the alphabet");
        assert_eq!(
            got.to_string(),
            "Invalid character in base32 input: \"ϑ\" at position 0"
        );
    }
}

/// `Invalid character in base32 input: "l" at position 11` → (`l`, 11).
fn split_message(message: &str) -> (String, usize) {
    let rest = message
        .strip_prefix("Invalid character in base32 input: \"")
        .expect("JS prefix");
    let (ch, position) = rest.rsplit_once("\" at position ").expect("JS tail");
    (ch.to_string(), position.parse().expect("index"))
}
