use super::*;

fn one(s: &str) -> QsValue {
    QsValue::One(s.to_owned())
}

fn kv(k: &str, v: QsValue) -> (String, QsValue) {
    (k.to_owned(), v)
}

#[test]
fn parses_pairs() {
    assert_eq!(
        parse_query("a=1&a=2&b"),
        vec![
            kv("a", QsValue::Many(vec!["1".into(), "2".into()])),
            kv("b", one(""))
        ]
    );
    assert_eq!(parse_query("=x"), vec![kv("", one("x"))]);
    assert_eq!(parse_query("&&a"), vec![kv("a", one(""))]);
    assert_eq!(parse_query("a+b=c+d"), vec![kv("a b", one("c d"))]);
    assert_eq!(parse_query("a=b=c"), vec![kv("a", one("b=c"))]);
}

#[test]
fn three_repeats_stay_one_list() {
    let q = parse_query("a=1&a=2&a=3");
    assert_eq!(
        get(&q, "a"),
        Some(&QsValue::Many(vec!["1".into(), "2".into(), "3".into()]))
    );
    assert_eq!(get(&q, "z"), None);
}

#[test]
fn soft_decoding() {
    assert_eq!(parse_query("n=%zz%41"), vec![kv("n", one("%zzA"))]);
    assert_eq!(parse_query("n=%E0%A4%A"), vec![kv("n", one("\u{FFFD}%A"))]);
    assert_eq!(parse_query("k=%2"), vec![kv("k", one("%2"))]);
}

#[test]
fn limits_to_1000_keys() {
    let s: Vec<String> = (0..1005).map(|i| format!("k{i}=v")).collect();
    assert_eq!(parse_query(&s.join("&")).len(), 1000);
}
