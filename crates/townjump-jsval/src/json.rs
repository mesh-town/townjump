// JS: `JSON.parse` and `JSON.stringify` (ECMAScript, V8 error texts).
// Related: json_parse.rs (parsing), json_error.rs (V8 texts), value.rs (`JsValue::parse_json`), record.rs.
use crate::{error::JsError, number::number_to_string, value::JsValue};

/// `JSON.parse(s)`.
pub fn parse(s: &str) -> Result<JsValue, JsError> {
    crate::json_parse::parse(s)
}

/// `JSON.stringify(v)`; `None` means the result is `undefined` (for values JSON cannot express).
pub fn stringify(v: &JsValue) -> Option<String> {
    Some(match v {
        JsValue::Null => "null".into(),
        JsValue::Bool(b) => b.to_string(),
        JsValue::Number(n) => number_json(*n),
        JsValue::String(s) => quote(s),
        JsValue::Array(items) => format!("[{}]", join(items.iter().map(element))),
        JsValue::Object(pairs) => format!("{{{}}}", join(pairs.iter().map(member))),
    })
}

fn number_json(n: f64) -> String {
    if n.is_finite() {
        return number_to_string(n);
    }
    "null".into()
}

fn element(v: &JsValue) -> String {
    stringify(v).unwrap_or_else(|| "null".into())
}

fn member((key, v): &(String, JsValue)) -> String {
    format!("{}:{}", quote(key), element(v))
}

fn join(parts: impl Iterator<Item = String>) -> String {
    parts.collect::<Vec<_>>().join(",")
}

/// `QuoteJSONString`: only `"`, `\`, control characters < 0x20.
fn quote(s: &str) -> String {
    let body: String = s.chars().map(escape).collect();
    format!("\"{body}\"")
}

fn escape(c: char) -> String {
    match c {
        '"' => "\\\"".into(),
        '\\' => "\\\\".into(),
        '\u{8}' => "\\b".into(),
        '\u{c}' => "\\f".into(),
        '\n' => "\\n".into(),
        '\r' => "\\r".into(),
        '\t' => "\\t".into(),
        c if (c as u32) < 0x20 => format!("\\u{:04x}", c as u32),
        c => c.to_string(),
    }
}

#[cfg(test)]
#[path = "json_tests.rs"]
mod tests;
