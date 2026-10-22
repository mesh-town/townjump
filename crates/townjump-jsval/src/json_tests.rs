// Tests for `json::parse` and `json::stringify`: V8 `JSON.parse`/`JSON.stringify`; expectations are from node 24.4, error texts verbatim.
// Server records come from dht-record-signing.json (valueHex); the broken record is cli-matrix.json, conn-record-badjson.
use super::*;
use townjump_testkit::vectors::load_value;

mod parse_valid {
    use super::*;

    #[test]
    fn numbers_follow_ieee_double() {
        // node 24.4, JSON.parse(text)
        let cases = [
            ("0", 0.0),
            ("3000", 3000.0),
            ("3000.5", 3000.5),
            ("1e3", 1000.0),
            ("1E+2", 100.0),
            ("0.1", 0.1),
            ("-1.5e-3", -0.0015),
            ("12345678901234567890", 1.2345678901234567e19),
            ("1e400", f64::INFINITY),
        ];
        for (text, want) in cases {
            assert_eq!(parse(text), Ok(n(want)), "JSON.parse({text:?})");
        }
    }

    #[test]
    fn minus_zero_keeps_its_sign() {
        match parse("-0") {
            Ok(JsValue::Number(x)) => {
                assert!(x == 0.0 && x.is_sign_negative(), "-0, got {x:?}")
            }
            other => panic!("expected Number(-0), got {other:?}"),
        }
    }

    #[test]
    fn literals_and_whitespace() {
        assert_eq!(parse("null"), Ok(JsValue::Null));
        assert_eq!(parse("false"), Ok(JsValue::Bool(false)));
        assert_eq!(parse(" \t\r\n[ ]\n"), Ok(JsValue::Array(vec![])));
        assert_eq!(parse("{}"), Ok(JsValue::Object(vec![])));
    }

    #[test]
    fn string_escapes_are_decoded() {
        assert_eq!(
            parse(r#""a\/b\b\f\n\r\t\"\\""#),
            Ok(s("a/b\u{8}\u{c}\n\r\t\"\\"))
        );
        assert_eq!(parse(r#""Aé😀""#), Ok(s("Aé😀")), "UTF-8 without escaping");
    }

    #[test]
    fn unicode_escapes_are_decoded() {
        // node 24.4, JSON.parse('"\\u00e9"') and a surrogate pair
        assert_eq!(parse(r#""é""#), Ok(s("é")));
        assert_eq!(parse(r#""😀""#), Ok(s("😀")), "surrogate pair");
        assert_eq!(parse(r#""A\u0000""#), Ok(s("A\u{0}")));
    }

    #[test]
    fn nested_containers_keep_order() {
        let got = parse(r#"{"z":[1,{"b":null}],"a":true}"#);
        let inner = JsValue::Object(vec![("b".into(), JsValue::Null)]);
        let want = JsValue::Object(vec![
            ("z".into(), JsValue::Array(vec![n(1.0), inner])),
            ("a".into(), JsValue::Bool(true)),
        ]);
        assert_eq!(got, Ok(want));
    }

    #[test]
    fn duplicate_key_keeps_first_position_and_last_value() {
        // node 24.4, JSON.stringify(JSON.parse('{"a":1,"b":2,"a":3}')) === '{"a":3,"b":2}'
        let v = parse(r#"{"a":1,"b":2,"a":3}"#).expect("valid JSON");
        assert_eq!(stringify(&v), Some(r#"{"a":3,"b":2}"#.to_string()));
    }
}

mod parse_errors {
    use super::*;

    #[test]
    fn every_case_gives_the_v8_message() {
        for (text, message) in SYNTAX_ERRORS {
            let want = JsError::Syntax((*message).to_string());
            assert_eq!(parse(text), Err(want), "JSON.parse({text:?})");
        }
    }

    #[test]
    fn long_input_is_cut_at_the_start() {
        // node 24.4, JSON.parse('x'.repeat(100)).message
        let want =
            JsError::Syntax("Unexpected token 'x', \"xxxxxxxxxx\"... is not valid JSON".into());
        assert_eq!(parse(&"x".repeat(100)), Err(want));
    }

    #[test]
    fn error_at_the_end_of_long_input_shows_the_tail() {
        // node 24.4, JSON.parse('[' + '1,'.repeat(40) + ']').message
        let text = format!("[{}]", "1,".repeat(40));
        let want =
            JsError::Syntax("Unexpected token ']', ...\"1,1,1,1,1,]\" is not valid JSON".into());
        assert_eq!(parse(&text), Err(want));
    }

    #[test]
    fn error_in_the_middle_of_long_input_shows_both_sides() {
        // node 24.4, JSON.parse('[' + '1,'.repeat(20) + 'x' + ',1'.repeat(20) + ']').message
        let text = format!("[{}x{}]", "1,".repeat(20), ",1".repeat(20));
        let want = JsError::Syntax(
            "Unexpected token 'x', ...\"1,1,1,1,1,x,1,1,1,1,\"... is not valid JSON".into(),
        );
        assert_eq!(parse(&text), Err(want));
    }
}

mod stringify {
    use super::*;

    #[test]
    fn scalars_use_javascript_text() {
        // node 24.4, JSON.stringify(x)
        let cases = [
            (JsValue::Null, "null"),
            (JsValue::Bool(true), "true"),
            (n(3000.0), "3000"),
            (n(3000.5), "3000.5"),
            (n(0.30000000000000004), "0.30000000000000004"),
            (n(-0.0), "0"),
            (n(1e21), "1e+21"),
            (n(f64::NAN), "null"),
            (n(f64::INFINITY), "null"),
            (n(f64::NEG_INFINITY), "null"),
        ];
        check(&cases);
    }

    #[test]
    fn strings_are_escaped() {
        let cases = [
            (s("abc"), r#""abc""#),
            (s(""), r#""""#),
            (
                s("a\"b\\c\n\t\r\u{8}\u{c}\u{1}\u{1f}"),
                r#""a\"b\\c\n\t\r\b\f\u0001\u001f""#,
            ),
            (s("\u{7f}é😀\u{2028}"), "\"\u{7f}é😀\u{2028}\""),
        ];
        check(&cases);
    }

    #[test]
    fn containers_have_no_spaces() {
        let obj = JsValue::Object(vec![
            ("b".into(), n(1.0)),
            ("a".into(), JsValue::Array(vec![s("x"), JsValue::Null])),
        ]);
        let cases = [
            (JsValue::Array(vec![]), "[]"),
            (JsValue::Object(vec![]), "{}"),
            (obj, r#"{"b":1,"a":["x",null]}"#),
        ];
        check(&cases);
    }

    #[test]
    fn object_keys_are_escaped_like_strings() {
        // node 24.4, JSON.stringify({ 'a"b\n': 1, '': 2 })
        let v = JsValue::Object(vec![("a\"b\n".into(), n(1.0)), (String::new(), n(2.0))]);
        assert_eq!(stringify(&v), Some(r#"{"a\"b\n":1,"":2}"#.to_string()));
    }

    fn check(cases: &[(JsValue, &str)]) {
        for (v, want) in cases {
            assert_eq!(
                stringify(v),
                Some((*want).to_string()),
                "JSON.stringify({v:?})"
            );
        }
    }
}

mod records_from_vectors {
    use super::*;

    #[test]
    fn compact_texts_survive_parse_then_stringify() {
        let texts = record_texts();
        assert!(
            texts.len() >= 10,
            "too few records in the vectors: {}",
            texts.len()
        );
        for text in texts {
            let v = parse(&text).unwrap_or_else(|e| panic!("record {text:?}: {e}"));
            assert_eq!(stringify(&v), Some(text.clone()), "record {text:?}");
        }
    }
}

// node 24.4, try { JSON.parse(text) } catch (e) { e.message }
const SYNTAX_ERRORS: &[(&str, &str)] = &[
    ("", "Unexpected end of JSON input"),
    (" ", "Unexpected end of JSON input"),
    ("\t", "Unexpected end of JSON input"),
    ("nul", "Unexpected end of JSON input"),
    ("tru", "Unexpected end of JSON input"),
    ("[", "Unexpected end of JSON input"),
    ("[1,", "Unexpected end of JSON input"),
    (r#"{"a":"#, "Unexpected end of JSON input"),
    (
        "not json",
        "Unexpected token 'o', \"not json\" is not valid JSON",
    ),
    ("undefined", "\"undefined\" is not valid JSON"),
    ("NaN", "\"NaN\" is not valid JSON"),
    ("Infinity", "\"Infinity\" is not valid JSON"),
    ("[,]", "Unexpected token ',', \"[,]\" is not valid JSON"),
    ("'a'", "Unexpected token ''', \"'a'\" is not valid JSON"),
    ("+1", "Unexpected token '+', \"+1\" is not valid JSON"),
    (".5", "Unexpected token '.', \".5\" is not valid JSON"),
    (
        r#"{"a":}"#,
        "Unexpected token '}', \"{\"a\":}\" is not valid JSON",
    ),
    (
        "{",
        "Expected property name or '}' in JSON at position 1 (line 1 column 2)",
    ),
    (
        r#"{"a""#,
        "Expected ':' after property name in JSON at position 4 (line 1 column 5)",
    ),
    (
        "{\"a\":1,\n\"b\" 2}",
        "Expected ':' after property name in JSON at position 12 (line 2 column 5)",
    ),
    (
        r#"{"a":1"#,
        "Expected ',' or '}' after property value in JSON at position 6 (line 1 column 7)",
    ),
    (
        r#"{"a":1 "b":2}"#,
        "Expected ',' or '}' after property value in JSON at position 7 (line 1 column 8)",
    ),
    (
        r#"{"a":1,}"#,
        "Expected double-quoted property name in JSON at position 7 (line 1 column 8)",
    ),
    (
        "[1",
        "Expected ',' or ']' after array element in JSON at position 2 (line 1 column 3)",
    ),
    (
        "[1 2]",
        "Expected ',' or ']' after array element in JSON at position 3 (line 1 column 4)",
    ),
    ("[1,]", "Unexpected token ']', \"[1,]\" is not valid JSON"),
    (
        "01",
        "Unexpected number in JSON at position 1 (line 1 column 2)",
    ),
    (
        "nul1",
        "Unexpected number in JSON at position 3 (line 1 column 4)",
    ),
    (
        "-",
        "No number after minus sign in JSON at position 1 (line 1 column 2)",
    ),
    (
        "1.",
        "Unterminated fractional number in JSON at position 2 (line 1 column 3)",
    ),
    (
        "1e",
        "Exponent part is missing a number in JSON at position 2 (line 1 column 3)",
    ),
    (
        "\"abc",
        "Unterminated string in JSON at position 4 (line 1 column 5)",
    ),
    (
        "\"a\nb\"",
        "Bad control character in string literal in JSON at position 2 (line 1 column 3)",
    ),
    (
        r#""\x""#,
        "Bad escaped character in JSON at position 2 (line 1 column 3)",
    ),
    (
        r#""\u12""#,
        "Bad Unicode escape in JSON at position 5 (line 1 column 6)",
    ),
    (
        r#""\u{41}""#,
        "Bad Unicode escape in JSON at position 3 (line 1 column 4)",
    ),
    (
        "1 2",
        "Unexpected non-whitespace character after JSON at position 2 (line 1 column 3)",
    ),
    (
        "truex",
        "Unexpected non-whitespace character after JSON at position 4 (line 1 column 5)",
    ),
    (
        r#"{"a":1}}"#,
        "Unexpected non-whitespace character after JSON at position 7 (line 1 column 8)",
    ),
    ("κλωξ", "Unexpected token 'κ', \"κλωξ\" is not valid JSON"),
];

/// Server record texts (`{…}` without spaces) from the `valueHex` of the dht-record-signing.json vector.
fn record_texts() -> Vec<String> {
    let signing = load_value("holesail/dht-record-signing.json");
    let rows = signing["signing"].as_array().expect("signing");
    let hex = rows.iter().filter_map(|r| r["valueHex"].as_str());
    let texts = hex.map(|h| String::from_utf8(unhex(h)).expect("valueHex — UTF-8"));
    texts.filter(|t| t.starts_with('{')).collect()
}

fn unhex(h: &str) -> Vec<u8> {
    let digits = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).expect("hex");
    (0..h.len()).step_by(2).map(digits).collect()
}

fn s(x: &str) -> JsValue {
    JsValue::String(x.to_string())
}

fn n(x: f64) -> JsValue {
    JsValue::Number(x)
}
