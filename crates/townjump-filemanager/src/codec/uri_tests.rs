use super::*;

fn ok(s: &str) -> String {
    decode_uri_component(s).unwrap()
}

#[test]
fn decodes_utf8_and_keeps_plus() {
    assert_eq!(ok("%CF%80"), "π");
    assert_eq!(ok("%F0%9F%98%80"), "😀");
    assert_eq!(ok("%2e%2e%2f"), "../");
    assert_eq!(ok("a+b"), "a+b");
}

#[test]
fn rejects_malformed() {
    for s in [
        "%",
        "%zz",
        "%E0%A4%A",
        "%C0%AE",
        "%ED%A0%80",
        "%F4%90%80%80",
        "%u0041",
    ] {
        assert_eq!(decode_uri_component(s), Err(UriMalformed), "{s}");
    }
}
