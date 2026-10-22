// JS: testvectors/holesail-cli/mock-dht.cjs (variables MOCK_RECORD, MOCK_RECORD_FOR, MOCK_FAKE_CONN).
// Related: shared.rs stores the factory settings, dht.rs takes the record stub and the `connect` mode from them;
// gates and injected errors (`hold_*`, `failing_*`) go beyond the JS mock, knobs.rs executes them.
use std::collections::HashMap;

/// For which key the prepared record is served.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecordFor {
    Any,
    Key([u8; 32]),
}

impl RecordFor {
    fn matches(&self, pk: &[u8; 32]) -> bool {
        match self {
            RecordFor::Any => true,
            RecordFor::Key(key) => key == pk,
        }
    }
}

/// Mock settings; strict by default: no stub, no fake connection, `connect` to a foreign key is an error.
#[derive(Clone, Debug, Default)]
pub struct MockConfig {
    pub(crate) record: Option<(RecordFor, Vec<u8>)>,
    pub(crate) fake_conn: bool,
    pub(crate) lenient_connect: bool,
    pub(crate) hold_listen: bool,
    pub(crate) hold_put: bool,
    pub(crate) listen_error: Option<String>,
    pub(crate) put_error_from: Option<usize>,
}

impl MockConfig {
    /// Stub for `mutable_get` (`seq` 3) for keys matching `target`.
    pub fn record(mut self, target: RecordFor, value: impl Into<Vec<u8>>) -> Self {
        self.record = Some((target, value.into()));
        self
    }

    /// `server.listen` immediately yields an incoming connection from `[9; 32]` (like `MOCK_FAKE_CONN`).
    pub fn fake_conn(mut self, on: bool) -> Self {
        self.fake_conn = on;
        self
    }

    /// `connect` to an unregistered key yields a stub instead of `PeerNotFound` (like the JS mock).
    pub fn lenient_connect(mut self, on: bool) -> Self {
        self.lenient_connect = on;
        self
    }

    /// `server.listen` does not complete until `MockFactory::release_listen` is called.
    pub fn hold_listen(mut self, on: bool) -> Self {
        self.hold_listen = on;
        self
    }

    /// `mutable_put` does not complete until `MockFactory::release_put` is called.
    pub fn hold_put(mut self, on: bool) -> Self {
        self.hold_put = on;
        self
    }

    /// `server.listen` immediately answers `DhtError::Backend(text)`.
    pub fn failing_listen(mut self, text: impl Into<String>) -> Self {
        self.listen_error = Some(text.into());
        self
    }

    /// `mutable_put` number `call` (counting from 1) and all following ones answer `DhtError::Backend("put failed")`.
    pub fn failing_put_from(mut self, call: usize) -> Self {
        self.put_error_from = Some(call);
        self
    }

    /// The stub for `pk`, if there is one and it matches.
    pub(crate) fn canned(&self, pk: &[u8; 32]) -> Option<&[u8]> {
        let (target, value) = self.record.as_ref()?;
        target.matches(pk).then_some(value.as_slice())
    }

    /// Settings from "name, value" pairs of environment variables; always `lenient_connect`, as in the JS mock.
    pub fn from_env_pairs<'a>(vars: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        let env: HashMap<&str, &str> = vars.into_iter().collect();
        let fake_conn = env.get("MOCK_FAKE_CONN").is_some_and(|v| !v.is_empty());
        let base = MockConfig::default()
            .lenient_connect(true)
            .fake_conn(fake_conn);
        let target = env.get("MOCK_RECORD_FOR").and_then(|v| parse_record_for(v));
        match (target, env.get("MOCK_RECORD")) {
            (Some(target), Some(value)) if !value.is_empty() => base.record(target, *value),
            _ => base,
        }
    }
}

/// `*` - any key; exactly 64 `[0-9a-f]` characters - that key; anything else the JS mock would match to no key.
fn parse_record_for(text: &str) -> Option<RecordFor> {
    if text == "*" {
        return Some(RecordFor::Any);
    }
    let lowercase_hex = text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    if text.len() != 64 || !lowercase_hex {
        return None;
    }
    hex::decode(text).ok()?.try_into().ok().map(RecordFor::Key)
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
