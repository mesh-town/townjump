// JS: `HyperDHT` arguments and results (`createServer` opts, `connect` opts, `mutableGet` result).
// Related: traits.rs (where these types appear in signatures), incoming.rs (incoming queue).
use crate::duplex::Duplex;
use std::fmt;
use std::sync::Arc;

/// Established connection: the peer key from the handshake, and the stream.
pub struct DhtConnection {
    pub remote_public_key: [u8; 32],
    pub stream: Box<dyn Duplex>,
    /// JS: `destroy()` tears down a real stream and sends `close`; a fake stream does not send `close`.
    reports_close_on_drop: bool,
}

impl DhtConnection {
    /// Regular connection: dropping the stream for the consumer is a `close` event.
    pub fn new(remote_public_key: [u8; 32], stream: Box<dyn Duplex>) -> Self {
        DhtConnection {
            remote_public_key,
            stream,
            reports_close_on_drop: true,
        }
    }

    /// Stub stream, like the fake one in JS: dropping the stream does not produce `close`.
    #[must_use]
    pub fn silent_close(mut self) -> Self {
        self.reports_close_on_drop = false;
        self
    }

    /// Whether dropping the stream produces a `close` event (yes by default).
    pub fn reports_close_on_drop(&self) -> bool {
        self.reports_close_on_drop
    }
}

impl fmt::Debug for DhtConnection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DhtConnection")
            .field("remote_public_key", &self.remote_public_key)
            .finish_non_exhaustive()
    }
}

/// Mutable record value: `seq` and bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MutableRecord {
    pub seq: u64,
    pub value: Vec<u8>,
}

/// Firewall callback: gets the client public key, `true` = reject (as in JS).
pub type Firewall = Arc<dyn Fn(&[u8; 32]) -> bool + Send + Sync>;

/// `createServer` options: `firewall: None` lets everyone in (JS `false`).
#[derive(Clone, Default)]
pub struct ServerOpts {
    pub firewall: Option<Firewall>,
    pub reusable_socket: bool,
    /// Public key of a blind-relay node to pair through; `None` = no relay (the JS holesail default).
    pub relay_through: Option<[u8; 32]>,
}

/// `connect` options: `false` ↔ JS without options (UDP), the trace has no `opts`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ConnectOpts {
    pub reusable_socket: bool,
    /// Public key of a blind-relay node to pair through; `None` = no relay (the JS holesail default).
    pub relay_through: Option<[u8; 32]>,
}

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "types_close_tests.rs"]
mod close_tests;
