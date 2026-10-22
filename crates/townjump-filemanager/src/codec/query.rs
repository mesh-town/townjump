// JS: node/lib/querystring (parse, '+' -> space, lenient decoding, limit of 1000 pairs).
use super::uri::{decode_uri_component, hex_val};

const MAX_PAIRS: usize = 1000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum QsValue {
    One(String),
    Many(Vec<String>),
}

pub(crate) fn parse_query(s: &str) -> Vec<(String, QsValue)> {
    let mut out: Vec<(String, QsValue)> = Vec::new();
    for pair in s.split('&').take(MAX_PAIRS) {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        push(&mut out, unescape(k), unescape(v));
    }
    out
}

fn push(out: &mut Vec<(String, QsValue)>, k: String, v: String) {
    let Some(slot) = out.iter_mut().find(|(key, _)| *key == k) else {
        out.push((k, QsValue::One(v)));
        return;
    };
    if let QsValue::Many(list) = &mut slot.1 {
        list.push(v);
        return;
    }
    let QsValue::One(first) = std::mem::replace(&mut slot.1, QsValue::Many(Vec::new())) else {
        return;
    };
    slot.1 = QsValue::Many(vec![first, v]);
}

fn unescape(s: &str) -> String {
    let s = s.replace('+', " ");
    if !s.contains('%') {
        return s;
    }
    decode_uri_component(&s).unwrap_or_else(|_| lossy_unescape(&s))
}

/// querystring.unescapeBuffer: `%XX` -> byte, everything else as is; then UTF-8 with replacement.
fn lossy_unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let pair = (i + 2 < b.len() && b[i] == b'%')
            .then(|| hex_val(b[i + 1]).zip(hex_val(b[i + 2])))
            .flatten();
        if let Some((h, l)) = pair {
            out.push(h << 4 | l);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub(crate) fn get<'a>(q: &'a [(String, QsValue)], key: &str) -> Option<&'a QsValue> {
    q.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

#[cfg(test)]
#[path = "query_tests.rs"]
mod tests;
