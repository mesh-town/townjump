// JS: minimist/index.js (value types), holesail/src/index.js (`Boolean(v)`, `String(v)`, `v.key`).
// Related: number.rs (Number#toString), json.rs (JSON.parse/stringify), error.rs (exceptions).
use crate::{error::JsError, number::number_to_string};

/// A JS value as it comes from argv and from the JSON of a DHT record. "No value" (`undefined`) is `None` outside.
#[derive(Debug, Clone, PartialEq)]
pub enum JsValue {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<JsValue>),
    Object(Vec<(String, JsValue)>),
}

impl JsValue {
    /// `Boolean(v)`.
    pub fn truthy(&self) -> bool {
        match self {
            JsValue::Null => false,
            JsValue::Bool(b) => *b,
            JsValue::Number(n) => !(*n == 0.0 || n.is_nan()),
            JsValue::String(s) => !s.is_empty(),
            JsValue::Array(_) | JsValue::Object(_) => true,
        }
    }

    /// `String(v)`.
    pub fn to_js_string(&self) -> String {
        match self {
            JsValue::Null => "null".into(),
            JsValue::Bool(b) => b.to_string(),
            JsValue::Number(n) => number_to_string(*n),
            JsValue::String(s) => s.clone(),
            JsValue::Array(items) => join_elements(items),
            JsValue::Object(_) => "[object Object]".into(),
        }
    }

    /// `JSON.parse(s)`; the error text is as in V8.
    pub fn parse_json(s: &str) -> Result<JsValue, JsError> {
        crate::json::parse(s)
    }

    /// `v.key`: `TypeError` for `Null`, a missing key gives `Ok(None)`.
    pub fn get(&self, key: &str) -> Result<Option<&JsValue>, JsError> {
        match self {
            JsValue::Null => Err(JsError::Type(format!(
                "Cannot read properties of null (reading '{key}')"
            ))),
            JsValue::Object(pairs) => Ok(pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v)),
            _ => Ok(None),
        }
    }
}

/// `Array.prototype.join`: `null` gives an empty string.
fn join_elements(items: &[JsValue]) -> String {
    let text = |v: &JsValue| match v {
        JsValue::Null => String::new(),
        other => other.to_js_string(),
    };
    items.iter().map(text).collect::<Vec<_>>().join(",")
}

#[cfg(test)]
#[path = "value_tests.rs"]
mod tests;
