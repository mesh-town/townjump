use super::disposition_value;

/// A Latin Extended-A character (U+0130..U+014F) in the name is truncated to its low byte, as Node does with non-Latin-1 characters.
#[test]
fn latin1_truncation_of_latin_ext_name() {
    assert_eq!(
        disposition_value("ŀŃŁŁĺĸĹ ńİĹĻ.txt", 3).as_deref(),
        Some("attachment; filename=\"@CAA:89 D09;.txt\"")
    );
    assert_eq!(
        disposition_value("ńİĹĻ.txt", 19).as_deref(),
        Some("attachment; filename=\"D09;.txt\"")
    );
}

#[test]
fn ascii_name_unchanged() {
    assert_eq!(
        disposition_value("q\"uote.txt", 2).as_deref(),
        Some("attachment; filename=\"q\"uote.txt\"")
    );
}

/// Empty file (`_contentLength` = 0): no truncation, the non-Latin-1 letter U+0444 fails the regexp.
#[test]
fn empty_file_keeps_cyrillic_and_fails() {
    assert_eq!(disposition_value("φ.txt", 0), None);
    // A Latin-1 character at len = 0 passes (written as byte 0xE9).
    assert_eq!(
        disposition_value("é", 0).as_deref(),
        Some("attachment; filename=\"é\"")
    );
}

/// Truncated bytes that do not form utf8 give U+FFFD -> not allowed (`é` at len > 0).
#[test]
fn truncated_invalid_utf8_fails() {
    assert_eq!(disposition_value("é", 5), None);
}

/// A control character in the name is not allowed at any `len`.
#[test]
fn control_char_fails() {
    assert_eq!(disposition_value("a\nb", 5), None);
}
