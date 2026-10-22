use super::*;

#[test]
fn format_bytes_table() {
    let cases: [(f64, &str); 16] = [
        (0.0, "0 Bytes"),
        (1.0, "1 Bytes"),
        (1023.0, "1023 Bytes"),
        (1024.0, "1 KB"),
        (1152.0, "1.13 KB"),
        (1056.0, "1.03 KB"),
        (1536.0, "1.5 KB"),
        (307200.0, "300 KB"),
        (999999.0, "976.56 KB"),
        (10485761.0, "10 MB"),
        (1099511627776.0, "1 TB"),
        (1048575.0, "1024 KB"),
        (1073741823.0, "1024 MB"),
        (1073741824.0, "1 GB"),
        (1125899906842624.0, "1 PB"),
        (10.0, "10 Bytes"),
    ];
    for (b, want) in cases {
        assert_eq!(format_bytes(b), want, "bytes={b}");
    }
}

#[test]
fn parse_int10_accepts_prefix() {
    for s in [" 5", "\u{a0}5", "+5", "5x"] {
        assert_eq!(parse_int10(s), 5.0, "{s:?}");
    }
    assert_eq!(parse_int10("1e3"), 1.0);
    assert_eq!(parse_int10("0x10"), 0.0);
}

#[test]
fn parse_int10_nan() {
    assert!(parse_int10("").is_nan());
    assert!(parse_int10("abc").is_nan());
    assert!(parse_int10("-").is_nan());
}

#[test]
fn escape_html_five_chars() {
    assert_eq!(escape_html("&<>\"'"), "&amp;&lt;&gt;&quot;&#039;");
    assert_eq!(escape_html("a b"), "a b");
}

#[test]
fn replace_first_once() {
    assert_eq!(replace_first("a.b.c", ".", ""), "ab.c");
    assert_eq!(replace_first("abc", "x", ""), "abc");
}

#[test]
fn js_num_prints_like_js() {
    assert_eq!(js_num(2.0), "2");
    assert_eq!(js_num(f64::NAN), "NaN");
    assert_eq!(js_num(-0.0), "0");
    assert_eq!(js_num(0.5), "0.5");
    assert_eq!(js_num(1e21), "1e+21");
    assert_eq!(js_num(1.5e-7), "1.5e-7");
    assert_eq!(js_num(123456789012.0), "123456789012");
}

#[test]
fn ct_eq_compares_bytes_and_lengths() {
    assert!(ct_eq(b"s3cret", b"s3cret"));
    assert!(ct_eq(b"", b""));
    assert!(!ct_eq(b"s3cret", b"s3creT"));
    assert!(!ct_eq(b"s3cret", b"s3cre"));
    assert!(!ct_eq(b"", b"x"));
}
