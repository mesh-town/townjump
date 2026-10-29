// JS: `HyperDHT` arguments and results (`createServer` opts, `connect` opts, `mutableGet` result).
// Related: traits.rs (where these types appear in signatures), incoming.rs (incoming queue).
use crate::duplex::Duplex;
use crate::relay::{HolepunchFilter, RelayAddress, RelayPath, RelayThrough};
use std::fmt;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;

/// Reads the smoothed round-trip time of a live connection; `None` before the first sample and once the
/// connection is gone.
pub type RttProbe = Arc<dyn Fn() -> Option<Duration> + Send + Sync>;

/// Established connection: the peer key from the handshake, and the stream.
pub struct DhtConnection {
    pub remote_public_key: [u8; 32],
    pub stream: Box<dyn Duplex>,
    /// JS: `destroy()` tears down a real stream and sends `close`; a fake stream does not send `close`.
    reports_close_on_drop: bool,
    remote_addr: Option<SocketAddr>,
    rtt_probe: Option<RttProbe>,
    relay_path: Option<watch::Receiver<RelayPath>>,
}

impl DhtConnection {
    /// Regular connection: dropping the stream for the consumer is a `close` event.
    pub fn new(remote_public_key: [u8; 32], stream: Box<dyn Duplex>) -> Self {
        DhtConnection {
            remote_public_key,
            stream,
            reports_close_on_drop: true,
            remote_addr: None,
            rtt_probe: None,
            relay_path: None,
        }
    }

    /// Sets the peer address reported by [`DhtConnection::remote_addr`].
    #[must_use]
    pub fn with_remote_addr(mut self, addr: Option<SocketAddr>) -> Self {
        self.remote_addr = addr;
        self
    }

    /// Address of the peer, if the backend knows it. Outgoing: the address the stream was set up with (without a
    /// relay the first non-private address from the server's answer, otherwise the dialed one). Incoming: the
    /// address the handshake came from (not necessarily the peer's DHT socket, so `Dht::connect_direct` to it is
    /// not guaranteed). Through a blind relay: the relay's address. `None` for backends without addresses.
    pub fn remote_addr(&self) -> Option<SocketAddr> {
        self.remote_addr
    }

    /// Attaches the probe read by [`DhtConnection::rtt`].
    #[must_use]
    pub fn with_rtt_probe(mut self, probe: RttProbe) -> Self {
        self.rtt_probe = Some(probe);
        self
    }

    /// Smoothed round-trip time of the connection; `None` without a probe, before the first sample, or once the
    /// connection is gone.
    pub fn rtt(&self) -> Option<Duration> {
        self.rtt_probe.as_ref().and_then(|probe| probe())
    }

    /// The probe behind [`DhtConnection::rtt`]. Keep it to read the RTT after moving `stream` out of the
    /// connection.
    pub fn rtt_probe(&self) -> Option<RttProbe> {
        self.rtt_probe.clone()
    }

    /// Attaches the relay path label read by [`DhtConnection::relay_path`].
    #[must_use]
    pub fn with_relay_path(mut self, path: watch::Receiver<RelayPath>) -> Self {
        self.relay_path = Some(path);
        self
    }

    /// Live path label of a connection set up through a relay; `None` for a connection without relay.
    pub fn relay_path(&self) -> Option<watch::Receiver<RelayPath>> {
        self.relay_path.clone()
    }

    /// Whether the data currently flows through a relay; `false` without relay.
    pub fn is_relayed(&self) -> bool {
        self.relay_path
            .as_ref()
            .is_some_and(|rx| *rx.borrow() == RelayPath::Relayed)
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
            .field("remote_addr", &self.remote_addr)
            .field(
                "relay_path",
                &self.relay_path.as_ref().map(|rx| *rx.borrow()),
            )
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
    /// Blind relay(s) to pair through; `None` = no relay (the JS holesail default).
    pub relay_through: Option<RelayThrough>,
    /// Addresses of relays, so they are reached without a DHT lookup.
    pub relay_addresses: Vec<RelayAddress>,
    /// Holepunch veto; `None` lets every punch run.
    pub holepunch: Option<HolepunchFilter>,
    /// Keep-alive of the relay connection; `None` = backend default.
    pub relay_keep_alive: Option<Duration>,
}

impl fmt::Debug for ServerOpts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServerOpts")
            .field("firewall", &self.firewall.is_some())
            .field("reusable_socket", &self.reusable_socket)
            .field("relay_through", &self.relay_through)
            .field("relay_addresses", &self.relay_addresses)
            .field("holepunch", &self.holepunch.is_some())
            .field("relay_keep_alive", &self.relay_keep_alive)
            .finish()
    }
}

/// `connect` options: `false` ↔ JS without options (UDP), the trace has no `opts`.
#[derive(Clone)]
pub struct ConnectOpts {
    pub reusable_socket: bool,
    /// Blind relay(s) to pair through; `None` = no relay (the JS holesail default).
    pub relay_through: Option<RelayThrough>,
    /// Addresses of relays, so they are reached without a DHT lookup.
    pub relay_addresses: Vec<RelayAddress>,
    /// Use LAN shortcuts to a peer behind the same NAT (JS default `true`).
    pub local_connection: bool,
    /// Holepunch veto; `None` lets every punch run.
    pub holepunch: Option<HolepunchFilter>,
    /// Keep-alive of the relay connection; `None` = backend default.
    pub relay_keep_alive: Option<Duration>,
    /// Test lever: wait this long before the client starts holepunching (only with a relay).
    #[doc(hidden)]
    pub punch_delay: Option<Duration>,
}

impl Default for ConnectOpts {
    fn default() -> Self {
        ConnectOpts {
            reusable_socket: false,
            relay_through: None,
            relay_addresses: Vec::new(),
            local_connection: true,
            holepunch: None,
            relay_keep_alive: None,
            punch_delay: None,
        }
    }
}

impl fmt::Debug for ConnectOpts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectOpts")
            .field("reusable_socket", &self.reusable_socket)
            .field("relay_through", &self.relay_through)
            .field("relay_addresses", &self.relay_addresses)
            .field("local_connection", &self.local_connection)
            .field("holepunch", &self.holepunch.is_some())
            .field("relay_keep_alive", &self.relay_keep_alive)
            .field("punch_delay", &self.punch_delay)
            .finish()
    }
}

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "types_path_tests.rs"]
mod path_tests;

#[cfg(test)]
#[path = "types_close_tests.rs"]
mod close_tests;
