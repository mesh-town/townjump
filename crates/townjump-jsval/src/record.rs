// JS: holesail-server/index.js:start (`JSON.stringify({ host, udp, port })`), the `??` operator.
// Related: json.rs (serialisation), value.rs (`JsValue`).
use crate::value::JsValue;

/// `a ?? b`: `Null` and "no value" yield, `0` and `''` are kept.
pub fn nullish<'a>(a: Option<&'a JsValue>, b: Option<&'a JsValue>) -> Option<&'a JsValue> {
    match a {
        Some(JsValue::Null) | None => b,
        present => present,
    }
}

/// `JSON.stringify({ host, udp, port })`: exactly this key order, `undefined` does not reach the JSON.
pub fn record_json(
    host: Option<&JsValue>,
    udp: Option<&JsValue>,
    port: Option<&JsValue>,
) -> String {
    let fields = [("host", host), ("udp", udp), ("port", port)];
    let present = fields
        .into_iter()
        .filter_map(|(k, v)| v.map(|v| (k.to_string(), v.clone())))
        .collect();
    crate::json::stringify(&JsValue::Object(present)).unwrap_or_default()
}

#[cfg(test)]
#[path = "record_tests.rs"]
mod tests;
