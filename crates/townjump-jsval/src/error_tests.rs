// Tests for `JsError::name`: V8/Node exception class; the "Name: text" header is as Node prints it (cli-matrix.json, conn-record-badjson).
use super::*;

mod name {
    use super::*;

    #[test]
    fn type_error_is_type_error() {
        let e = JsError::Type("Cannot read properties of null (reading 'port')".into());
        assert_eq!(e.name(), "TypeError");
    }

    #[test]
    fn syntax_error_is_syntax_error() {
        let e = JsError::Syntax("Unexpected end of JSON input".into());
        assert_eq!(e.name(), "SyntaxError");
    }

    #[test]
    fn range_error_is_range_error_whatever_the_code() {
        for code in ["ERR_SOCKET_BAD_PORT", "ERR_OTHER"] {
            let e = JsError::Range {
                code: code.into(),
                message: "msg".into(),
            };
            assert_eq!(e.name(), "RangeError", "code {code}");
        }
    }
}

mod header {
    use super::*;

    #[test]
    fn syntax_error_reads_like_the_node_stack_line() {
        // cli-matrix.json, conn-record-badjson: «SyntaxError: Unexpected token 'o', "not json" is not valid JSON»
        let e = JsError::Syntax("Unexpected token 'o', \"not json\" is not valid JSON".into());
        assert_eq!(
            header(&e),
            "SyntaxError: Unexpected token 'o', \"not json\" is not valid JSON"
        );
    }

    #[test]
    fn type_error_reads_like_the_node_stack_line() {
        // node 24.4, try { null.port } catch (e) { `${e.name}: ${e.message}` }
        let e = JsError::Type("Cannot read properties of null (reading 'port')".into());
        assert_eq!(
            header(&e),
            "TypeError: Cannot read properties of null (reading 'port')"
        );
    }

    #[test]
    fn range_error_reads_like_the_node_message_line() {
        // node 24.4, validatePort(65536): `${e.name}: ${e.message}`
        let e = JsError::Range {
            code: "ERR_SOCKET_BAD_PORT".into(),
            message: "Port should be >= 0 and < 65536. Received type number (65536).".into(),
        };
        assert_eq!(
            header(&e),
            "RangeError: Port should be >= 0 and < 65536. Received type number (65536)."
        );
    }
}

fn header(e: &JsError) -> String {
    format!("{}: {e}", e.name())
}
