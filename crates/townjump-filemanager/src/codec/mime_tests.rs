use super::content_type;

#[test]
fn known_and_unknown() {
    assert_eq!(content_type(".html"), "text/html");
    assert_eq!(content_type(".7z"), "application/x-7z-compressed");
    assert_eq!(content_type(".cpp"), "text/x-c++src");
    assert_eq!(content_type(".zzz"), "application/octet-stream");
    assert_eq!(content_type(""), "application/octet-stream");
}

#[test]
fn lookup_is_case_sensitive_caller_lowercases() {
    assert_eq!(content_type(".HTML"), "application/octet-stream");
}
