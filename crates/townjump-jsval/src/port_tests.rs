// Tests for `to_port`: Node 24.4 `validatePort` (`node:internal/validators`, name "Port", allowZero) over a minimist value.
// RangeError ERR_SOCKET_BAD_PORT texts were captured by running Node and are written in verbatim.
use super::*;

mod accepted {
    use super::*;

    #[test]
    fn integer_numbers_in_range() {
        // node 24.4, validatePort(x)
        for (x, want) in [
            (0.0, 0),
            (-0.0, 0),
            (80.0, 80),
            (3000.0, 3000),
            (65535.0, 65535),
        ] {
            assert_eq!(to_port(&JsValue::Number(x)), Ok(want), "port {x:?}");
        }
    }

    #[test]
    fn numeric_strings_as_javascript_converts_them() {
        // node 24.4, validatePort(str): +str must be an integer 0..65535, surrounding whitespace is allowed
        let cases = [
            ("0", 0),
            ("80", 80),
            ("080", 80),
            ("00", 0),
            ("+0", 0),
            ("+80", 80),
            ("0x10", 16),
            ("0XfF", 255),
            ("0b11", 3),
            ("0o17", 15),
            ("1e3", 1000),
            ("1e2", 100),
            (".5e2", 50),
            ("5.", 5),
            ("6.5535e4", 65535),
            (" 80 ", 80),
            ("80 ", 80),
            ("\t80\n", 80),
        ];
        for (text, want) in cases {
            assert_eq!(to_port(&s(text)), Ok(want), "port {text:?}");
        }
    }
}

mod rejected_numbers {
    use super::*;

    #[test]
    fn out_of_range_fractional_and_non_finite() {
        // node 24.4, validatePort(x).message; the second element is `String(x)` in the message parentheses
        let cases = [
            (65536.0, "65536"),
            (99999.0, "99999"),
            (4294967296.0, "4294967296"),
            (3000.5, "3000.5"),
            (-1.0, "-1"),
            (f64::NAN, "NaN"),
            (f64::INFINITY, "Infinity"),
            (1e21, "1e+21"),
            (1e-7, "1e-7"),
            (0.1, "0.1"),
        ];
        for (x, shown) in cases {
            let want = bad_port(&format!("type number ({shown})"));
            assert_eq!(to_port(&JsValue::Number(x)), Err(want), "port {x:?}");
        }
    }
}

mod rejected_strings {
    use super::*;

    #[test]
    fn non_numeric_and_out_of_range_text() {
        // node 24.4, validatePort(str).message
        let cases = [
            "",
            " ",
            "abc",
            "-1",
            "65536",
            "3000.5",
            "Infinity",
            "-Infinity",
            "NaN",
            "٨٠",
            "8_0",
            "1e5",
            "0x",
            "0x10000",
            "+0x10",
        ];
        for text in cases {
            let want = bad_port(&format!("type string ('{text}')"));
            assert_eq!(to_port(&s(text)), Err(want), "port {text:?}");
        }
    }
}

mod rejected_other_types {
    use super::*;

    #[test]
    fn booleans_null_arrays_objects() {
        // node 24.4 (testvectors/probes-js/jsval.cjs, testvectors/holesail-cli/node-port-vectors.json): `net.connect({ port })` checks the type
        // before `validatePort` - TypeError ERR_INVALID_ARG_TYPE; RangeError ERR_SOCKET_BAD_PORT only for a number and a string.
        let cases = [
            (JsValue::Bool(true), "type boolean (true)"),
            (JsValue::Bool(false), "type boolean (false)"),
            (JsValue::Null, "null"),
            (JsValue::Array(vec![]), "an instance of Array"),
            (JsValue::Object(vec![]), "an instance of Object"),
        ];
        for (v, shown) in cases {
            assert_eq!(to_port(&v), Err(wrong_type(shown)), "port {v:?}");
        }
    }
}

fn bad_port(received: &str) -> JsError {
    JsError::Range {
        code: "ERR_SOCKET_BAD_PORT".into(),
        message: format!("Port should be >= 0 and < 65536. Received {received}."),
    }
}

fn wrong_type(received: &str) -> JsError {
    JsError::Type(format!(
        "The \"options.port\" property must be one of type number or string. Received {received}"
    ))
}

fn s(x: &str) -> JsValue {
    JsValue::String(x.to_string())
}
