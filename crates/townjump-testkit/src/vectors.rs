// JS: testvectors/gen/*.mjs (dump the `testvectors/**/*.json` files); regeneration - `node testvectors/gen/all.mjs`.
// Related: vectors_tests.rs (checks that all files open).
use serde::de::DeserializeOwned;
use serde_json::Value;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../testvectors/");

/// Vector file `rel` (for example `holesail/key-derivation.json`) as JSON without the top-level `_meta` key;
/// a missing file panics with a hint on how to get it.
pub fn load_value(rel: &str) -> Value {
    let path = format!("{ROOT}{rel}");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!("no vector file {path}: {e}; generate it: node testvectors/gen/all.mjs")
    });
    let mut value: Value =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("vector {path} is not JSON: {e}"));
    if let Some(object) = value.as_object_mut() {
        object.remove("_meta");
    }
    value
}

/// The same, deserialised into `T`.
pub fn load<T: DeserializeOwned>(rel: &str) -> T {
    serde_json::from_value(load_value(rel))
        .unwrap_or_else(|e| panic!("vector {rel} does not fit the type: {e}"))
}

#[cfg(test)]
#[path = "vectors_tests.rs"]
mod tests;
