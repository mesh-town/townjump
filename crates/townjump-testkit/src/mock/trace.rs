// JS: testvectors/holesail-cli/mock-dht.cjs (`trace` writes a JSON line per call), run-matrix.mjs (trace comparison).
// Related: events.rs builds the events, shared.rs holds the factory trace.
use crate::lock::lock;
use serde_json::Value;
use std::sync::{Arc, Mutex};

/// Events recorded by the mock; clones share one record.
#[derive(Clone, Default)]
pub struct MockTrace(Arc<Mutex<Vec<Value>>>);

impl MockTrace {
    pub fn events(&self) -> Vec<Value> {
        lock(&self.0).clone()
    }

    /// JSON lines as the JS mock writes them (the key order after `ev` differs - compare only `events()`).
    pub fn lines(&self) -> Vec<String> {
        self.events().iter().map(Value::to_string).collect()
    }

    /// Events with the name `ev`.
    pub fn named(&self, ev: &str) -> Vec<Value> {
        let is_named = |e: &&Value| e["ev"] == ev;
        self.events().iter().filter(is_named).cloned().collect()
    }

    pub(crate) fn push(&self, event: Value) {
        lock(&self.0).push(event);
    }
}

/// Removes from the JS matrix trace what the mock does not write: `HyperDHT.keyPair` and `randomBytes`.
pub fn strip_untracked(events: &[Value]) -> Vec<Value> {
    let untracked =
        |e: &&Value| matches!(e["ev"].as_str(), Some("HyperDHT.keyPair" | "randomBytes"));
    events.iter().filter(|e| !untracked(e)).cloned().collect()
}

#[cfg(test)]
#[path = "trace_tests.rs"]
mod tests;
