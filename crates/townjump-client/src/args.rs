// JS: the `connect(options)` argument in holesail-client/index.js (what `Holesail.connect` assembles).
// Related: client.rs (`HolesailClient::connect`).
use townjump_jsval::JsValue;

/// Client arguments; `None` means the flag is not set (merged with the DHT record via `??`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ConnectArgs {
    /// Local listen port (`--port`).
    pub port: Option<JsValue>,
    /// Local listen address (`--host`).
    pub host: Option<JsValue>,
    /// Value of the `--udp` flag as is.
    pub udp: Option<JsValue>,
}
