// Tests for `nullish` and `record_json`: `??` and `JSON.stringify({ host, udp, port })`; expectations are from node 24.4.
use super::*;

mod nullish_operator {
    use super::*;

    #[test]
    fn left_value_wins_when_present() {
        let (a, b) = (JsValue::String("a".into()), JsValue::String("b".into()));
        assert_eq!(nullish(Some(&a), Some(&b)), Some(&a));
    }

    #[test]
    fn falsy_but_not_nullish_left_is_kept() {
        let right = JsValue::String("r".into());
        for left in [
            JsValue::Number(0.0),
            JsValue::String(String::new()),
            JsValue::Bool(false),
            JsValue::Number(f64::NAN),
        ] {
            let got = nullish(Some(&left), Some(&right)).expect("left side is not nullish");
            assert!(
                std::ptr::eq(got, &left),
                "{left:?} ?? 'r' must return the left side"
            );
        }
    }

    #[test]
    fn null_left_falls_through_to_right() {
        let right = JsValue::Number(8989.0);
        assert_eq!(nullish(Some(&JsValue::Null), Some(&right)), Some(&right));
    }

    #[test]
    fn missing_left_falls_through_to_right() {
        let right = JsValue::Number(8989.0);
        assert_eq!(nullish(None, Some(&right)), Some(&right));
    }

    #[test]
    fn both_missing_gives_none() {
        assert_eq!(nullish(None, None), None);
    }

    #[test]
    fn null_right_is_returned_as_null() {
        // JS: `undefined ?? null === null` (testvectors/probes-js/jsval.cjs, `nullish_undefined_null`), hence Some(Null), not None.
        assert_eq!(nullish(None, Some(&JsValue::Null)), Some(&JsValue::Null));
    }
}

mod record_json {
    use super::*;

    #[test]
    fn keys_go_in_host_udp_port_order() {
        let got = rec(
            Some(s("127.0.0.1")),
            Some(JsValue::Bool(false)),
            Some(n(3000.0)),
        );
        assert_eq!(got, r#"{"host":"127.0.0.1","udp":false,"port":3000}"#);
    }

    #[test]
    fn all_undefined_gives_empty_object() {
        assert_eq!(rec(None, None, None), "{}");
    }

    #[test]
    fn undefined_fields_are_skipped() {
        assert_eq!(
            rec(None, Some(JsValue::Bool(true)), Some(n(80.0))),
            r#"{"udp":true,"port":80}"#
        );
        assert_eq!(
            rec(Some(s("h")), None, Some(n(80.0))),
            r#"{"host":"h","port":80}"#
        );
        assert_eq!(
            rec(Some(s("h")), Some(JsValue::Bool(false)), None),
            r#"{"host":"h","udp":false}"#
        );
    }

    #[test]
    fn null_is_written_as_null() {
        let got = rec(
            Some(JsValue::Null),
            Some(JsValue::Null),
            Some(JsValue::Null),
        );
        assert_eq!(got, r#"{"host":null,"udp":null,"port":null}"#);
    }

    #[test]
    fn numbers_use_javascript_text() {
        // node 24.4, JSON.stringify({port: x})
        let cases = [
            (3000.0, "3000"),
            (3000.5, "3000.5"),
            (16.0, "16"),
            (-0.0, "0"),
            (1e21, "1e+21"),
            (1e-7, "1e-7"),
        ];
        for (x, want) in cases {
            assert_eq!(
                rec(None, None, Some(n(x))),
                format!(r#"{{"port":{want}}}"#),
                "port {x:?}"
            );
        }
    }

    #[test]
    fn non_finite_numbers_become_null() {
        for x in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(
                rec(None, None, Some(n(x))),
                r#"{"port":null}"#,
                "port {x:?}"
            );
        }
    }

    #[test]
    fn strings_keep_their_type() {
        let got = rec(Some(s("")), Some(s("false")), Some(s("3000")));
        assert_eq!(got, r#"{"host":"","udp":"false","port":"3000"}"#);
    }

    #[test]
    fn arrays_and_objects_are_nested() {
        let port = JsValue::Array(vec![n(1.0), s("a"), JsValue::Null]);
        assert_eq!(rec(None, None, Some(port)), r#"{"port":[1,"a",null]}"#);
        let host = JsValue::Object(vec![("k".into(), JsValue::Bool(true))]);
        assert_eq!(rec(Some(host), None, None), r#"{"host":{"k":true}}"#);
    }

    #[test]
    fn strings_are_escaped_like_json_stringify() {
        // node 24.4, JSON.stringify({host: x})
        let cases = [
            ("a\"b\\c", r#"{"host":"a\"b\\c"}"#),
            ("a\nb\tc\rd", r#"{"host":"a\nb\tc\rd"}"#),
            ("\u{8}\u{c}", r#"{"host":"\b\f"}"#),
            ("\u{1}\u{1f}", r#"{"host":"\u0001\u001f"}"#),
            ("\u{7f}", "{\"host\":\"\u{7f}\"}"),
            ("\u{2028}", "{\"host\":\"\u{2028}\"}"),
            ("χοστ-😀", "{\"host\":\"χοστ-😀\"}"),
        ];
        for (host, want) in cases {
            assert_eq!(rec(Some(s(host)), None, None), want, "host {host:?}");
        }
    }
}

fn rec(host: Option<JsValue>, udp: Option<JsValue>, port: Option<JsValue>) -> String {
    record_json(host.as_ref(), udp.as_ref(), port.as_ref())
}

fn s(x: &str) -> JsValue {
    JsValue::String(x.to_string())
}

fn n(x: f64) -> JsValue {
    JsValue::Number(x)
}
