// JS: the `start(args)` argument in holesail-server/index.js (what `Holesail.connect` assembles).
// Related: server.rs (`HolesailServer::start`).
use townjump_jsval::JsValue;

/// Server start arguments; flag values are as from minimist, without type coercion.
#[derive(Debug, Clone, PartialEq)]
pub struct ServerArgs {
    /// Target port (`--live`).
    pub port: JsValue,
    /// Target address (`--host`, default `127.0.0.1`).
    pub host: JsValue,
    /// `hex(sha256(K))`; `None` means the server takes 32 random bytes from the OS (`getrandom`); the facade and CLI always pass `Some`.
    pub seed_hex: Option<String>,
    /// Private mode (`args.secure === true`).
    pub secure: bool,
    /// Value of the `--udp` flag as is; its truthiness selects the TCP/UDP branch.
    pub udp: Option<JsValue>,
}
