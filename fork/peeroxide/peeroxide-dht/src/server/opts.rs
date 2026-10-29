// JS: hyperdht/lib/server.js
//! Server options and the firewall callback (JS `opts.firewall`).

use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::time::Duration;

use crate::hyperdht::{HolepunchFilter, KeyPair, RelayAddress, RelayThrough};
use crate::hyperdht_messages::NoisePayload;
use crate::messages::Ipv4Peer;

/// Firewall callback: returns `true` to refuse the connecting client.
///
/// It is called synchronously, exactly once per new handshake (repeats of the
/// same Noise message are answered from the stored reply). A panic inside the
/// callback counts as a refusal and does not stop the server.
pub type Firewall = Arc<dyn Fn(&FirewallInput<'_>) -> bool + Send + Sync>;

/// What the firewall callback learns about the connecting client.
#[derive(Debug, Clone, Copy)]
pub struct FirewallInput<'a> {
    /// Static public key of the client (from the Noise handshake).
    pub remote_public_key: &'a [u8; 32],
    /// Decrypted Noise payload of the client (firewall type, addresses, UDX info).
    pub remote_payload: &'a NoisePayload,
    /// Address of the client as seen by the DHT (relay's view, or the sender itself).
    pub client_address: &'a Ipv4Peer,
}

/// Options of the DHT server (`Server`).
///
/// Built from `ServerOpts::default()` by assigning fields (the struct is
/// `#[non_exhaustive]`, so a literal is not available outside the crate).
#[non_exhaustive]
#[derive(Clone)]
pub struct ServerOpts {
    /// Firewall callback; `None` accepts every client.
    pub firewall: Option<Firewall>,
    /// Advertise the server socket as reusable (`udx.reusable_socket`).
    pub reusable_socket: bool,
    /// Blind-relay node to announce in every reply (JS `opts.relayThrough`); `None` announces none.
    pub relay_through: Option<RelayThrough>,
    /// Known addresses of the relays in `relay_through`; the relay handshake goes straight to them.
    pub relay_through_addresses: Vec<RelayAddress>,
    /// Key pair used towards the relay (JS `dht.defaultKeyPair`); `None` generates a fresh one.
    pub relay_key_pair: Option<KeyPair>,
    /// Keep-alive of the relay connection (JS `opts.relayKeepAlive`, 5000 ms).
    pub relay_keep_alive: Duration,
    /// Veto on holepunching (JS `opts.holepunch`); `None` lets every punch run.
    pub holepunch: Option<HolepunchFilter>,
}

impl Default for ServerOpts {
    fn default() -> Self {
        Self {
            firewall: None,
            reusable_socket: false,
            relay_through: None,
            relay_through_addresses: Vec::new(),
            relay_key_pair: None,
            relay_keep_alive: Duration::from_millis(5000),
            holepunch: None,
        }
    }
}

impl ServerOpts {
    /// Returns the options with the firewall callback set.
    pub fn with_firewall(mut self, firewall: Firewall) -> Self {
        self.firewall = Some(firewall);
        self
    }
}

impl fmt::Debug for ServerOpts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let callback = self.firewall.as_ref().map(|_| "<callback>");
        f.debug_struct("ServerOpts")
            .field("firewall", &callback)
            .field("reusable_socket", &self.reusable_socket)
            .field("relay_through", &self.relay_through)
            .field("relay_through_addresses", &self.relay_through_addresses)
            .field("relay_key_pair", &self.relay_key_pair.is_some())
            .field("relay_keep_alive", &self.relay_keep_alive)
            .field("holepunch", &self.holepunch.as_ref().map(|_| "<callback>"))
            .finish()
    }
}

/// `true` when the client must be refused: no callback means accept, a panic means refuse.
pub(super) fn rejects(opts: &ServerOpts, input: &FirewallInput<'_>) -> bool {
    rejects_with(opts.firewall.as_ref(), input)
}

/// Same as `rejects` for a bare callback (used by the relay service).
pub(crate) fn rejects_with(firewall: Option<&Firewall>, input: &FirewallInput<'_>) -> bool {
    let Some(firewall) = firewall else {
        return false;
    };
    catch_unwind(AssertUnwindSafe(|| firewall(input))).unwrap_or(true)
}

#[cfg(test)]
#[path = "opts_tests.rs"]
mod tests;
