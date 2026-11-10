use super::*;

fn p(s: &str) -> (String, Option<bool>) {
    let r = parse_url(s);
    (r.key, r.secure)
}

#[test]
fn prefix_with_private_marker() {
    assert_eq!(p("hs://s000abc"), ("abc".into(), Some(true)));
    assert_eq!(p("hs://sXYZabc"), ("abc".into(), Some(true)));
}

#[test]
fn prefix_without_private_marker() {
    assert_eq!(p("hs://0000abc"), ("abc".into(), None));
    assert_eq!(p("hs://S000abc"), ("abc".into(), None));
}

#[test]
fn short_prefix_keeps_whole_text() {
    assert_eq!(p("hs://"), ("hs://".into(), None));
    assert_eq!(p("hs://s00"), ("hs://s00".into(), Some(true)));
    assert_eq!(p("hs://s000"), (String::new(), Some(true)));
}

#[test]
fn bare_key_flag_from_sixth_unit() {
    assert_eq!(p("abcdes"), ("abcdes".into(), Some(true)));
    assert_eq!(p("abcdefs"), ("abcdefs".into(), None));
    assert_eq!(p("HS://s000abc"), ("HS://s000abc".into(), Some(true)));
    assert_eq!(p(""), (String::new(), None));
}

#[test]
fn utf16_positions() {
    assert_eq!(p("hs://😀000abc"), ("0abc".into(), None));
    assert_eq!(p("hs://s😀abc"), ("bc".into(), Some(true)));
    assert_eq!(p("hs://s000κλωξ"), ("κλωξ".into(), Some(true)));
}

#[test]
fn broken_pair_gives_replacement_char() {
    assert_eq!(p("hs://s00😀abc").0, "\u{fffd}abc");
}

#[test]
fn format_both_modes() {
    assert_eq!(format_url("abc", true), "hs://s000abc");
    assert_eq!(format_url("abc", false), "hs://0000abc");
    assert_eq!(format_url("", true), "hs://s000");
}

#[test]
fn format_then_parse_roundtrip() {
    assert_eq!(p(&format_url("abc", true)), ("abc".into(), Some(true)));
    assert_eq!(p(&format_url("abc", false)), ("abc".into(), None));
}
