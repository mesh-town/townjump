// JS: the `info` getter in holesail-server/index.js:HolesailServer (fields, `state` values).
// Related: server.rs (`HolesailServer::info`); the holesail facade adds `url`.
use crate::server::Running;
use townjump_dht::Dht;
use townjump_jsval::JsValue;
use townjump_keys::z32;

/// Server snapshot. The `state` and `protocol` strings are as in JS.
#[derive(Debug, Clone, PartialEq)]
pub struct ServerInfo {
    /// `type`: always `server`.
    pub kind: &'static str,
    /// `None` (before `start`), `listening`, `paused`, `destroyed`.
    pub state: Option<&'static str>,
    pub secure: bool,
    pub port: JsValue,
    pub host: JsValue,
    /// `udp` or `tcp` (by truthiness of `args.udp`).
    pub protocol: &'static str,
    /// `hex(seed)` from `args.seed`.
    pub seed: Option<String>,
    /// `z32(seed)` in private mode, otherwise `z32(public_key)`.
    pub key: String,
    /// `z32(public_key)`.
    pub public_key: String,
}

impl ServerInfo {
    /// Snapshot of a server that has not been started yet.
    pub(crate) fn unstarted(state: Option<&'static str>) -> Self {
        ServerInfo {
            kind: "server",
            state,
            secure: false,
            port: JsValue::Null,
            host: JsValue::Null,
            protocol: "tcp",
            seed: None,
            key: String::new(),
            public_key: String::new(),
        }
    }

    pub(crate) fn of<D: Dht>(
        running: &Running<D>,
        state: Option<&'static str>,
        key: String,
    ) -> Self {
        let args = &running.args;
        let udp = args.udp.as_ref().is_some_and(JsValue::truthy);
        ServerInfo {
            kind: "server",
            state,
            secure: args.secure,
            port: args.port.clone(),
            host: args.host.clone(),
            protocol: if udp { "udp" } else { "tcp" },
            seed: args.seed_hex.clone(),
            key,
            public_key: z32::encode(&running.key_pair.public_key),
        }
    }
}
