// Tests of `cli::argv`: 149 reference minimist 1.2.8 vectors (grouped by JS quirks) and targeted minimist quirk cases.
// The vectors are `testvectors/holesail-cli/minimist-vectors.json`; anything missing was captured by running minimist in JS.
use super::*;
use serde_json::{Value, json};
use townjump_jsval::{JsError, JsValue};
use townjump_testkit::vectors::load_value;

const PROTO_MESSAGE: &str = "(aliases[key] || []).forEach is not a function";

mod positional {
    use super::*;

    #[test]
    fn bare_words_stay_in_the_underscore_array() {
        check_vectors(&[0, 1, 61, 63]);
    }

    #[test]
    fn double_dash_ends_flag_parsing() {
        check_vectors(&[59, 60, 62]);
    }

    #[test]
    fn flag_takes_the_next_word_and_the_rest_is_positional() {
        check_vectors(&[71, 72]);
    }

    #[test]
    fn unicode_dash_is_not_a_flag() {
        check_vectors(&[112]);
    }

    #[test]
    fn q5_numeric_positionals_become_numbers() {
        check_vectors(&[73, 74]);
    }

    #[test]
    fn underscore_is_the_first_key_of_the_result() {
        let parsed = parse(&["--live", "3000", "--udp=1", "x"]).expect("parse");
        let JsValue::Object(entries) = parsed else {
            panic!("an object was expected")
        };
        assert_eq!(entries[0].0, "_", "the `_` key comes first");
        assert_eq!(
            entries[0].1,
            JsValue::Array(vec![JsValue::String("x".into())])
        );
    }

    #[test]
    fn underscore_flag_name_is_a_positional_alias() {
        check_extra(&["--_", "x"], json!({ "_": ["x"] }));
    }
}

mod flag_values {
    use super::*;

    #[test]
    fn long_flag_takes_the_next_word_or_equals_value_or_is_true() {
        check_vectors(&[
            12, 13, 25, 26, 28, 29, 56, 57, 58, 64, 65, 81, 83, 92, 93, 94, 95, 102, 103, 104, 105,
            107,
        ]);
    }

    #[test]
    fn negative_number_after_a_flag_is_a_new_flag() {
        check_vectors(&[7, 20, 27, 53, 54, 55, 82, 88]);
    }

    #[test]
    fn repeated_flag_collects_an_array() {
        check_vectors(&[30, 31, 80]);
        check_extra(&["--a", "1", "--a"], json!({ "_": [], "a": [1, true] }));
        check_extra(
            &["--k=v", "--k=w", "--k=x"],
            json!({ "_": [], "k": ["v", "w", "x"] }),
        );
    }

    #[test]
    fn no_prefix_negates_the_flag() {
        check_vectors(&[36, 37]);
        check_extra(&["--a", "--no-a"], json!({ "_": [], "a": false }));
        check_extra(&["--no-a.b"], json!({ "_": [], "a": { "b": false } }));
    }

    #[test]
    fn dotted_names_nest_objects() {
        check_vectors(&[75, 76]);
        check_extra(
            &["--a.b=1", "--a=2"],
            json!({ "_": [], "a": [{ "b": 1 }, 2] }),
        );
    }

    #[test]
    fn prototype_pollution_paths_are_ignored() {
        check_vectors(&[77, 78]);
        check_extra(&["--x.__proto__.y=1"], json!({ "_": [], "x": {} }));
        check_extra(&["--toString.x=1"], json!({ "_": [] }));
    }

    #[test]
    fn flag_names_are_case_and_punctuation_sensitive() {
        check_vectors(&[108, 109, 110, 111]);
    }

    #[test]
    fn dotted_path_through_a_scalar_is_a_type_error() {
        let err = parse(&["--a", "1", "--a.b=2"]).expect_err("minimist crashes: `1` not an object");
        assert!(
            matches!(err, JsError::Type(_)),
            "TypeError expected, not {err:?}"
        );
    }
}

mod short_flags {
    use super::*;

    #[test]
    fn cluster_sets_every_letter_true() {
        check_vectors(&[44, 45, 46, 49, 52]);
    }

    #[test]
    fn last_letter_takes_the_value() {
        check_vectors(&[43, 47, 48, 50, 51]);
        check_extra(&["-ab=c"], json!({ "_": [], "a": true, "b": "c" }));
    }

    #[test]
    fn digits_are_flag_letters() {
        check_vectors(&[53, 54, 55]);
        check_extra(&["-3.5"], json!({ "_": [], "3": 0.5 }));
        check_extra(
            &["-x5y"],
            json!({ "_": [], "x": true, "5": true, "y": true }),
        );
    }

    #[test]
    fn q3_dash_v_is_a_plain_flag_and_never_becomes_version() {
        let parsed = parse(&["-v"]).expect("parse");
        assert_eq!(
            sorted(parsed),
            sorted(from_json(&json!({ "_": [], "v": true })))
        );
    }

    #[test]
    fn q3_dash_h_with_a_word_keeps_the_word_under_h_not_help() {
        let parsed = parse(&["-h", "live"]).expect("parse");
        assert_eq!(
            sorted(parsed),
            sorted(from_json(&json!({ "_": [], "h": "live" })))
        );
    }
}

mod numbers {
    use super::*;

    #[test]
    fn q5_numeric_strings_become_numbers() {
        check_vectors(&[
            2, 3, 4, 5, 6, 8, 9, 18, 19, 21, 22, 23, 24, 40, 41, 47, 48, 66, 67, 68, 69, 70, 85,
            86, 89, 90, 91, 96, 97, 98, 100, 106,
        ]);
    }

    #[test]
    fn q5_forty_digit_key_is_rounded_to_a_double() {
        let parsed = parse(&["--key", "0123456789012345678901234567890123456789"]).expect("parse");
        assert_eq!(
            get(&parsed, "key"),
            Some(&JsValue::Number(1.2345678901234568e38))
        );
    }

    #[test]
    fn q5_leading_zeros_are_dropped_from_a_connect_key() {
        let parsed = parse(&["--connect", "00443214"]).expect("parse");
        assert_eq!(get(&parsed, "connect"), Some(&JsValue::Number(443214.0)));
    }

    #[test]
    fn q5_hex_and_exponent_forms_are_numbers() {
        check_extra(&["--k", "0X1F"], json!({ "_": [], "k": 31 }));
        check_extra(&["--k=1e-3"], json!({ "_": [], "k": 0.001 }));
        check_extra(&["--k", "+.5e+2"], json!({ "_": [], "k": 50 }));
        check_extra(&["--k", "1e5"], json!({ "_": [], "k": 100000 }));
    }

    #[test]
    fn q5_overflowing_exponent_is_infinity() {
        let parsed = parse(&["--live", "1e400"]).expect("parse");
        assert_eq!(get(&parsed, "live"), Some(&JsValue::Number(f64::INFINITY)));
    }

    #[test]
    fn q5_lookalikes_stay_strings() {
        check_vectors(&[10, 11, 12, 13, 14, 15, 16, 17, 28, 87, 99, 113]);
        check_extra(&["--k", "1E3"], json!({ "_": [], "k": "1E3" }));
        check_extra(&["--k", "0x"], json!({ "_": [], "k": "0x" }));
    }

    #[test]
    fn q5_minus_infinity_is_a_flag_cluster() {
        let expected = json!({ "_": [], "k": true, "I": true, "n": true, "f": true, "i": true, "t": true, "y": true });
        check_extra(&["--k", "-Infinity"], expected);
    }
}

mod booleans {
    use super::*;

    #[test]
    fn bare_and_valued_boolean_flags() {
        check_vectors(&[32, 36, 37, 39, 101]);
    }

    #[test]
    fn q6_public_false_is_the_string_false() {
        let parsed = parse(&["--public=false"]).expect("parse");
        assert_eq!(
            get(&parsed, "public"),
            Some(&JsValue::String("false".into()))
        );
    }

    #[test]
    fn q6_public_word_false_is_the_string_false() {
        check_vectors(&[33, 34, 35, 38]);
    }

    #[test]
    fn q6_udp_false_is_the_string_false() {
        let parsed = parse(&["--udp=false"]).expect("parse");
        assert_eq!(get(&parsed, "udp"), Some(&JsValue::String("false".into())));
    }

    #[test]
    fn q6_udp_zero_is_the_number_zero() {
        let parsed = parse(&["--udp=0"]).expect("parse");
        assert_eq!(get(&parsed, "udp"), Some(&JsValue::Number(0.0)));
    }
}

mod prototype_names {
    use super::*;

    #[test]
    fn q12_proto_flag_with_a_value_is_a_type_error() {
        let err = parse(&["--__proto__", "x"]).expect_err("vector 79: minimist crashes");
        assert_eq!(err, JsError::Type(PROTO_MESSAGE.into()));
    }

    #[test]
    fn q12_every_object_prototype_name_throws_in_every_flag_form() {
        let names = [
            "constructor",
            "__defineGetter__",
            "__defineSetter__",
            "hasOwnProperty",
            "__lookupGetter__",
            "__lookupSetter__",
            "isPrototypeOf",
            "propertyIsEnumerable",
            "toString",
            "valueOf",
            "__proto__",
            "toLocaleString",
        ];
        for name in names {
            for argv in [
                vec![format!("--{name}"), "x".into()],
                vec![format!("--no-{name}")],
                vec![format!("--{name}=1")],
            ] {
                let err = parse_argv(argv.clone()).expect_err(&format!("{argv:?} must crash"));
                assert_eq!(err, JsError::Type(PROTO_MESSAGE.into()), "argv {argv:?}");
            }
        }
    }

    #[test]
    fn proto_word_as_a_positional_is_harmless() {
        check_extra(&["__proto__"], json!({ "_": ["__proto__"] }));
    }
}

#[test]
fn all_149_vectors_match_minimist() {
    let count = vectors().len();
    assert_eq!(
        count, 149,
        "the number of vectors in the file changed — check the groups above"
    );
    check_vectors(&(0..count).collect::<Vec<_>>());
}

fn vectors() -> Vec<Value> {
    let Value::Array(all) = load_value("holesail-cli/minimist-vectors.json") else {
        panic!("the minimist vectors file is not an array")
    };
    all
}

fn parse(argv: &[&str]) -> Result<JsValue, JsError> {
    parse_argv(argv.iter().map(|a| a.to_string()).collect())
}

fn get<'a>(parsed: &'a JsValue, key: &str) -> Option<&'a JsValue> {
    let JsValue::Object(entries) = parsed else {
        panic!("an object was expected, not {parsed:?}")
    };
    entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

fn check_extra(argv: &[&str], expected: Value) {
    let parsed = parse(argv).unwrap_or_else(|e| panic!("argv {argv:?}: error {e}"));
    assert_eq!(
        sorted(parsed),
        sorted(from_json(&expected)),
        "argv {argv:?}"
    );
}

fn check_vectors(ids: &[usize]) {
    let all = vectors();
    let failures: Vec<String> = ids
        .iter()
        .filter_map(|&id| vector_failure(id, &all[id]))
        .collect();
    assert!(
        failures.is_empty(),
        "vectors differ:\n{}",
        failures.join("\n")
    );
}

fn vector_failure(id: usize, vector: &Value) -> Option<String> {
    let words = vector["argv"].as_array().expect("vector argv is an array");
    let argv: Vec<String> = words
        .iter()
        .map(|a| a.as_str().expect("argv string").to_owned())
        .collect();
    let got = parse_argv(argv.clone());
    let expected = match vector.get("throws") {
        Some(message) => Err(JsError::Type(
            message.as_str().expect("error text").to_owned(),
        )),
        None => Ok(sorted(from_json(&vector["parsed"]))),
    };
    let got = got.map(sorted);
    (got != expected)
        .then(|| format!("  vector {id}: argv {argv:?}\n    want {expected:?}\n    got {got:?}"))
}

/// `__Infinity__` — how a vector writes `Infinity`, which JSON lacks.
fn from_json(v: &Value) -> JsValue {
    match v {
        Value::Null => JsValue::Null,
        Value::Bool(b) => JsValue::Bool(*b),
        Value::Number(n) => JsValue::Number(n.as_f64().expect("JSON number")),
        Value::String(s) if s == "__Infinity__" => JsValue::Number(f64::INFINITY),
        Value::String(s) => JsValue::String(s.clone()),
        Value::Array(a) => JsValue::Array(a.iter().map(from_json).collect()),
        Value::Object(o) => {
            JsValue::Object(o.iter().map(|(k, v)| (k.clone(), from_json(v))).collect())
        }
    }
}

/// JS has its own key order for objects (numeric names first), so we compare against sorted keys.
fn sorted(v: JsValue) -> JsValue {
    match v {
        JsValue::Array(a) => JsValue::Array(a.into_iter().map(sorted).collect()),
        JsValue::Object(o) => {
            let mut entries: Vec<_> = o.into_iter().map(|(k, v)| (k, sorted(v))).collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            JsValue::Object(entries)
        }
        other => other,
    }
}
