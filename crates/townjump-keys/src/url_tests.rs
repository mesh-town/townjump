// Tests for `parse_url` and `format_url`: url-parser.json (49 records), key-derivation.json and keyderivation-vectors.json (server url).
use super::*;
use crate::kd_vectors::{cli_rows, field, records};
use serde_json::Value;
use townjump_testkit::vectors::load_value;

mod parse_url {
    use super::*;

    #[test]
    fn every_vector_row_gives_key_and_secure_flag() {
        let file = load_value("holesail/url-parser.json");
        let rows = file["vectors"].as_array().expect("vectors");
        assert_eq!(rows.len(), 49, "the vector has 49 rows");
        for (i, row) in rows.iter().enumerate() {
            let text = input_text(&row["input"]);
            let got = parse_url(&text);
            let label = format!("record {i} of url-parser.json: input {text:?}");
            assert_eq!(got.key, row["key"].as_str().expect("key"), "{label}");
            assert_eq!(got.secure, row["secure"].as_bool(), "{label}");
        }
    }
}

mod bare_key {
    use super::*;

    #[test]
    fn sixth_char_s_marks_it_private_and_the_key_stays_whole() {
        // the flag "6th character equals `s`" is not tied to the `hs://` prefix
        assert_parses("abcdes", "abcdes", Some(true));
        assert_parses("abcdesX", "abcdesX", Some(true));
    }

    #[test]
    fn s_at_any_other_position_is_not_a_mark() {
        assert_parses("abcdef", "abcdef", None);
        assert_parses("abcdefs", "abcdefs", None);
        assert_parses("sabcde", "sabcde", None);
    }

    #[test]
    fn public_z32_key_with_s_in_the_sixth_place_reads_as_private() {
        // url-parser.json, records 43-45: random public keys, about 1/32 of them
        let key = "bpeuispw1jybm1ojymsoxrnr4qsf5xsngbzopfrcndpe7pzduhso";
        assert_parses(key, key, Some(true));
    }

    #[test]
    fn empty_string_gives_empty_key_and_no_mark() {
        assert_parses("", "", None);
    }
}

mod prefix {
    use super::*;

    #[test]
    fn s000_gives_private_and_strips_nine_chars() {
        assert_parses("hs://s000abc", "abc", Some(true));
        assert_parses("hs://s000", "", Some(true));
    }

    #[test]
    fn zeros_do_not_force_public_mode() {
        // parsing returns "not set", not `false`; the mode is decided by the flag
        assert_parses("hs://0000abc", "abc", None);
        assert_parses("hs://0000", "", None);
    }

    #[test]
    fn four_chars_after_the_scheme_are_dropped_unchecked() {
        assert_parses("hs://x000abc", "abc", None);
        assert_parses("hs://sXYZabc", "abc", Some(true));
        assert_parses("hs://S000abc", "abc", None);
    }

    #[test]
    fn only_the_first_prefix_is_removed() {
        assert_parses("hs://s000hs://s000x", "hs://s000x", Some(true));
    }

    #[test]
    fn shorter_than_nine_chars_is_no_prefix_but_still_marked_by_the_sixth_char() {
        assert_parses("hs://s00", "hs://s00", Some(true));
        assert_parses("hs://", "hs://", None);
    }

    #[test]
    fn scheme_is_case_sensitive() {
        // `HS://` is not a prefix: the key is the whole string, but the 6th character `s` still sets the flag
        assert_parses("HS://s000abc", "HS://s000abc", Some(true));
        assert_parses(" hs://s000abc", " hs://s000abc", None);
    }

    #[test]
    fn indexes_count_utf16_code_units() {
        // node 24.4: an emoji takes two `substring` code units, a Greek letter one
        assert_parses("hs://😀000abc", "0abc", None);
        assert_parses("hs://s😀abc", "bc", Some(true));
        assert_parses("hs://κλωξ-κλωξ", "-κλωξ", None);
        assert_parses("hs://s000κλωξ", "κλωξ", Some(true));
    }
}

mod format_url {
    use super::*;

    #[test]
    fn secure_and_public_prefixes() {
        assert_eq!(format_url("abc", true), "hs://s000abc");
        assert_eq!(format_url("abc", false), "hs://0000abc");
        assert_eq!(format_url("", true), "hs://s000");
    }

    #[test]
    fn server_urls_from_key_derivation_vector() {
        for (label, rec) in records("server") {
            let got = format_url(rec.res_str("infoKey"), rec.secure());
            assert_eq!(got, rec.res_str("url"), "{label}");
        }
    }

    #[test]
    fn cli_vector_urls() {
        for row in cli_rows("private") {
            let got = format_url(field(&row, "K"), true);
            assert_eq!(got, field(&row, "url"), "{row}");
        }
        for row in cli_rows("publicNoKey") {
            let got = format_url(field(&row, "publicKeyZ32"), false);
            assert_eq!(got, field(&row, "url"), "{row}");
        }
    }

    #[test]
    fn parse_gives_back_what_format_wrote() {
        for key in [
            "abc",
            "bxxinyn6mjjf31p9ssxtobak64wkcyjty3jo7qrweikkmm1c8u7y",
            "κλωξ",
        ] {
            let secure = parse_url(&format_url(key, true));
            assert_eq!(
                (secure.key.as_str(), secure.secure),
                (key, Some(true)),
                "secure {key:?}"
            );
            let public = parse_url(&format_url(key, false));
            assert_eq!(
                (public.key.as_str(), public.secure),
                (key, None),
                "public {key:?}"
            );
        }
    }
}

fn assert_parses(url: &str, key: &str, secure: Option<bool>) {
    let want = ParsedUrl {
        key: key.to_string(),
        secure,
    };
    assert_eq!(parse_url(url), want, "parse_url({url:?})");
}

/// `urlParser` input as `String(url || '')`: `undefined`, `null`, `''`, `0`, `false` give an empty string.
fn input_text(input: &Value) -> String {
    match &input["value"] {
        Value::String(text) => text.clone(),
        Value::Bool(true) => "true".to_string(),
        Value::Number(number) if number.as_f64() != Some(0.0) => number.to_string(),
        _ => String::new(),
    }
}
