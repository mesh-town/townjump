// JS: the `info` getter in holesail-client/index.js:HolesailClient (fields, `state` values).
// Related: client.rs (`HolesailClient::info`); the holesail facade adds `url` and `seed`.
use townjump_jsval::JsValue;

/// Client snapshot. The `state` and `protocol` strings are as in JS.
#[derive(Debug, Clone, PartialEq)]
pub struct ClientInfo {
    /// `type`: always `client`.
    pub kind: &'static str,
    /// `None`, `waiting`, `listening`, `paused`, `destroyed`.
    pub state: Option<&'static str>,
    pub secure: bool,
    /// Final listen port (after merging with the DHT record).
    pub port: JsValue,
    /// Final listen address.
    pub host: JsValue,
    /// `udp` or `tcp` (by truthiness of the final `udp`).
    pub protocol: &'static str,
    /// `key: seed`: `z32(sha256(K))` in private mode, otherwise the server z32 key.
    pub key: String,
    /// `z32(public_key)`.
    pub public_key: String,
}
