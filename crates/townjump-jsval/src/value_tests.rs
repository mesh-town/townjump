// Tests for `JsValue`: truthy, to_js_string, get, parse_json; expectations are from node 24.4 (`Boolean(x)`, `String(x)`, `x.port`),
// minimist-vectors.json vectors and dht-record-live.json (server record, protocol).
use super::*;
use serde_json::Value;
use townjump_testkit::vectors::load_value;

mod truthy {
    use super::*;

    #[test]
    fn falsy_values_are_false() {
        let falsy = [
            (JsValue::Null, "null"),
            (JsValue::Bool(false), "false"),
            (n(0.0), "0"),
            (n(-0.0), "-0"),
            (n(f64::NAN), "NaN"),
            (s(""), "''"),
        ];
        for (v, what) in falsy {
            assert!(!v.truthy(), "{what} is falsy in JS");
        }
    }

    #[test]
    fn everything_else_is_true() {
        let truthy = [
            (JsValue::Bool(true), "true"),
            (n(1.0), "1"),
            (n(-1.0), "-1"),
            (n(0.1), "0.1"),
            (n(f64::INFINITY), "Infinity"),
            (s("false"), "'false'"),
            (s("0"), "'0'"),
            (s(" "), "' '"),
            (JsValue::Array(vec![]), "[]"),
            (JsValue::Object(vec![]), "{}"),
        ];
        for (v, what) in truthy {
            assert!(v.truthy(), "{what} is truthy in JS");
        }
    }

    #[test]
    fn q6_string_false_from_minimist_is_true() {
        // minimist-vectors.json: `--public=false` and `--public false` give the string 'false', not a boolean
        let cases: [&[&str]; 2] = [&["--public=false"], &["--public", "false"]];
        for argv in cases {
            let v = minimist_flag(argv, "public");
            assert_eq!(v, s("false"), "vector {argv:?}");
            assert!(v.truthy(), "'false' is truthy, the server is public");
        }
    }

    #[test]
    fn no_public_from_minimist_is_false() {
        let v = minimist_flag(&["--no-public"], "public");
        assert_eq!(v, JsValue::Bool(false), "vector --no-public");
        assert!(!v.truthy());
    }

    #[test]
    fn q13_zero_from_minimist_is_false() {
        // `--live 00` gives the number 0: the `--live` branch is skipped, the help text
        let v = minimist_flag(&["--live", "00"], "live");
        assert_eq!(v, n(0.0), "vector --live 00");
        assert!(!v.truthy());
    }
}

mod to_js_string {
    use super::*;

    #[test]
    fn scalars_use_javascript_text() {
        let cases = [
            (JsValue::Null, "null"),
            (JsValue::Bool(true), "true"),
            (JsValue::Bool(false), "false"),
            (n(3000.0), "3000"),
            (n(3000.5), "3000.5"),
            (n(-0.0), "0"),
            (n(1e21), "1e+21"),
            (n(f64::NAN), "NaN"),
            (n(f64::INFINITY), "Infinity"),
            (n(f64::NEG_INFINITY), "-Infinity"),
            (s("abc"), "abc"),
            (s(""), ""),
        ];
        check(&cases);
    }

    #[test]
    fn arrays_join_elements_with_comma_and_empty_null() {
        let nested = JsValue::Array(vec![
            n(1.0),
            s("a"),
            JsValue::Null,
            JsValue::Array(vec![n(2.0), n(3.0)]),
        ]);
        let cases = [
            (JsValue::Array(vec![]), ""),
            (JsValue::Array(vec![JsValue::Null]), ""),
            (JsValue::Array(vec![JsValue::Array(vec![])]), ""),
            (JsValue::Array(vec![JsValue::Bool(true)]), "true"),
            (nested, "1,a,,2,3"),
        ];
        check(&cases);
    }

    #[test]
    fn objects_are_object_object() {
        let cases = [
            (JsValue::Object(vec![]), "[object Object]"),
            (obj(&[("a", n(1.0))]), "[object Object]"),
            (
                JsValue::Array(vec![JsValue::Null, JsValue::Object(vec![])]),
                ",[object Object]",
            ),
        ];
        check(&cases);
    }

    #[test]
    fn q5_numbers_from_minimist_print_as_javascript() {
        // key and port after minimist are `String(Number(text))`: leading zeros, hex and exponent disappear
        let long_seed = "1234567890123456789012345678901234567890123456789012345678901234";
        let cases: [(&[&str], &str, &str); 11] = [
            (&["--live", "03000"], "live", "3000"),
            (&["--live", "0x10"], "live", "16"),
            (&["--live", "1e3"], "live", "1000"),
            (&["--live", "3000.5"], "live", "3000.5"),
            (&["--live", ".5"], "live", "0.5"),
            (&["--live", "00"], "live", "0"),
            (
                &["--live", "99999999999999999999"],
                "live",
                "100000000000000000000",
            ),
            (&["--username", "007"], "username", "7"),
            (&["--password", "1e3"], "password", "1000"),
            (
                &["--key", "12345678901234567890123456789012"],
                "key",
                "1.2345678901234567e+31",
            ),
            (&["--key", long_seed], "key", "1.2345678901234568e+63"),
        ];
        for (argv, key, want) in cases {
            assert_eq!(minimist_flag(argv, key).to_js_string(), want, "{argv:?}");
        }
    }

    fn check(cases: &[(JsValue, &str)]) {
        for (v, want) in cases {
            assert_eq!(v.to_js_string(), *want, "String({v:?})");
        }
    }
}

mod get {
    use super::*;

    #[test]
    fn present_key_gives_value() {
        let v = obj(&[("host", s("h")), ("port", n(3000.0))]);
        assert_eq!(v.get("port"), Ok(Some(&n(3000.0))));
        assert_eq!(v.get("host"), Ok(Some(&s("h"))));
    }

    #[test]
    fn missing_key_gives_none() {
        let v = obj(&[("host", s("h"))]);
        assert_eq!(v.get("port"), Ok(None), "undefined in JS");
        assert_eq!(v.get("Host"), Ok(None), "keys are case-sensitive");
    }

    #[test]
    fn explicit_null_value_is_some_null() {
        let v = obj(&[("port", JsValue::Null)]);
        assert_eq!(v.get("port"), Ok(Some(&JsValue::Null)));
    }

    #[test]
    fn null_target_throws_type_error_naming_the_key() {
        // node 24.4, null.port / null.host
        let want = JsError::Type("Cannot read properties of null (reading 'port')".into());
        assert_eq!(JsValue::Null.get("port"), Err(want));
        let want = JsError::Type("Cannot read properties of null (reading 'host')".into());
        assert_eq!(JsValue::Null.get("host"), Err(want));
    }

    #[test]
    fn q12_record_null_in_dht_throws_on_first_read() {
        // client.md: `JSON.parse('null')` gives null, `dhtData.port` throws - exit code 1
        let record = JsValue::parse_json("null").expect("null is valid JSON");
        let want = JsError::Type("Cannot read properties of null (reading 'port')".into());
        assert_eq!(record.get("port"), Err(want));
    }

    #[test]
    fn primitives_and_arrays_have_no_such_property() {
        let targets = [
            JsValue::Bool(true),
            n(3000.0),
            s("abc"),
            JsValue::Array(vec![n(1.0)]),
        ];
        for v in &targets {
            assert_eq!(v.get("port"), Ok(None), "{v:?}.port === undefined");
        }
    }

    #[test]
    fn duplicate_key_in_json_keeps_last_value() {
        // node 24.4, JSON.parse('{"port":1,"port":2}').port
        let v = JsValue::parse_json(r#"{"port":1,"port":2}"#).expect("valid JSON");
        assert_eq!(v.get("port"), Ok(Some(&n(2.0))));
    }

    #[test]
    fn dotted_minimist_flag_is_reachable_key_by_key() {
        // minimist-vectors.json: `--x.y.z 1` → { x: { y: { z: 1 } } }
        let parsed = JsValue::parse_json(r#"{"_":[],"x":{"y":{"z":1}}}"#).expect("valid JSON");
        let x = parsed.get("x").expect("x").expect("x is present");
        let y = x.get("y").expect("y").expect("y is present");
        assert_eq!(y.get("z"), Ok(Some(&n(1.0))));
        let vector = minimist_flag(&["--x.y.z", "1"], "x");
        assert_eq!(x, &vector, "the same object that minimist gives");
    }
}

mod parse_json {
    use super::*;

    #[test]
    fn record_keeps_keys_and_value_types() {
        let got = JsValue::parse_json(r#"{"host":"127.0.0.1","udp":false,"port":3000}"#);
        let want = obj(&[
            ("host", s("127.0.0.1")),
            ("udp", JsValue::Bool(false)),
            ("port", n(3000.0)),
        ]);
        assert_eq!(got, Ok(want));
    }

    #[test]
    fn scalars_and_arrays_parse() {
        assert_eq!(JsValue::parse_json("null"), Ok(JsValue::Null));
        assert_eq!(JsValue::parse_json("true"), Ok(JsValue::Bool(true)));
        assert_eq!(JsValue::parse_json("3000.5"), Ok(n(3000.5)));
        assert_eq!(JsValue::parse_json(r#""a\nb""#), Ok(s("a\nb")));
        assert_eq!(
            JsValue::parse_json(r#"[1,"a",null]"#),
            Ok(JsValue::Array(vec![n(1.0), s("a"), JsValue::Null]))
        );
    }

    #[test]
    fn broken_text_is_syntax_error_with_v8_message() {
        // cli-matrix.json, conn-record-badjson: record `not json`
        let want = JsError::Syntax("Unexpected token 'o', \"not json\" is not valid JSON".into());
        assert_eq!(JsValue::parse_json("not json"), Err(want));
    }

    #[test]
    fn empty_text_is_unexpected_end() {
        // node 24.4, JSON.parse('').message; record `empty` in dht-record-signing.json
        let want = JsError::Syntax("Unexpected end of JSON input".into());
        assert_eq!(JsValue::parse_json(""), Err(want));
    }

    #[test]
    fn every_minimist_result_from_the_vectors_is_read_back() {
        // minimist value types: numbers, strings, booleans, arrays, nested objects. Key order is not compared.
        let file = load_value("holesail-cli/minimist-vectors.json");
        for row in file.as_array().expect("array of vectors") {
            let Some(parsed) = row.get("parsed") else {
                continue;
            };
            let got = JsValue::parse_json(&parsed.to_string());
            assert_eq!(
                got.map(canon),
                Ok(canon(from_serde(parsed))),
                "argv {}",
                row["argv"]
            );
        }
    }

    #[test]
    fn server_records_give_host_port_and_protocol_of_the_ping() {
        // dht-record-live.json: `ping` is what `HolesailClient.ping` derived from the record (`udp ? 'udp' : 'tcp'`)
        let file = load_value("holesail/dht-record-live.json");
        for case in file["cases"].as_array().expect("cases") {
            let text = case["steps"][0]["read"]["valueText"]
                .as_str()
                .expect("valueText");
            let record = JsValue::parse_json(text).expect("record is valid JSON");
            let ping = &case["ping"];
            assert_eq!(
                record.get("host"),
                Ok(Some(&from_serde(&ping["host"]))),
                "{}",
                case["label"]
            );
            assert_eq!(
                record.get("port"),
                Ok(Some(&from_serde(&ping["port"]))),
                "{}",
                case["label"]
            );
            assert_eq!(
                protocol_of(&record),
                ping["protocol"].as_str().expect("protocol"),
                "{}",
                case["label"]
            );
        }
    }
}

/// `result.udp ? 'udp' : 'tcp'` from `HolesailClient.ping`.
fn protocol_of(record: &JsValue) -> &'static str {
    match record.get("udp") {
        Ok(Some(udp)) if udp.truthy() => "udp",
        _ => "tcp",
    }
}

fn s(x: &str) -> JsValue {
    JsValue::String(x.to_string())
}

fn n(x: f64) -> JsValue {
    JsValue::Number(x)
}

fn obj(pairs: &[(&str, JsValue)]) -> JsValue {
    JsValue::Object(
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), v.clone()))
            .collect(),
    )
}

/// Value of flag `key` from the minimist vector row with this `argv` - as is, without our parsing.
fn minimist_flag(argv: &[&str], key: &str) -> JsValue {
    let file = load_value("holesail-cli/minimist-vectors.json");
    let want = serde_json::json!(argv);
    let rows = file.as_array().expect("array of vectors");
    let row = rows.iter().find(|r| r["argv"] == want);
    from_serde(&row.unwrap_or_else(|| panic!("no vector {argv:?}"))["parsed"][key])
}

fn from_serde(v: &Value) -> JsValue {
    match v {
        Value::Null => JsValue::Null,
        Value::Bool(b) => JsValue::Bool(*b),
        Value::Number(x) => n(x.as_f64().expect("number as f64")),
        Value::String(x) => s(x),
        Value::Array(items) => JsValue::Array(items.iter().map(from_serde).collect()),
        Value::Object(map) => JsValue::Object(
            map.iter()
                .map(|(k, x)| (k.clone(), from_serde(x)))
                .collect(),
        ),
    }
}

/// Objects with keys in alphabetical order: comparison does not depend on key order.
fn canon(v: JsValue) -> JsValue {
    match v {
        JsValue::Array(items) => JsValue::Array(items.into_iter().map(canon).collect()),
        JsValue::Object(mut pairs) => {
            pairs.sort_by(|a, b| a.0.cmp(&b.0));
            JsValue::Object(pairs.into_iter().map(|(k, x)| (k, canon(x))).collect())
        }
        other => other,
    }
}
