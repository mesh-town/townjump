// JS: hyperdht/lib/router.js (onpeerhandshake), hyperdht/lib/server.js (_onpeerhandshake)
//! Sockets and addresses of one PEER_HANDSHAKE: where the request arrived, from which socket
//! the server answers (JS `reply.socket`) and to whom a relayed answer goes. Tests: `handshake_route_tests.rs`.

use std::fmt;
use std::sync::{Arc, Mutex};

use libudx::UdxSocket;

use crate::hyperdht_messages::{MODE_FROM_RELAY, MODE_FROM_SECOND_RELAY};
use crate::messages::Ipv4Peer;

/// The socket the server wants its handshake answer sent from.
///
/// The server puts its hole-punch socket here before it answers; the router that
/// forwards the answer takes it (JS: `socket: h.replyFromPuncher ? h.puncher.socket : null`).
/// Empty means "the socket the request arrived on".
#[derive(Clone, Default)]
pub struct ReplyVia(Arc<Mutex<Option<UdxSocket>>>);

impl ReplyVia {
    /// The answer must leave from `socket`.
    pub fn set(&self, socket: UdxSocket) {
        if let Ok(mut slot) = self.0.lock() {
            *slot = Some(socket);
        }
    }

    /// The socket chosen by the server, if any.
    pub fn take(&self) -> Option<UdxSocket> {
        self.0.lock().ok()?.take()
    }
}

impl fmt::Debug for ReplyVia {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let set = self.0.lock().is_ok_and(|slot| slot.is_some());
        f.debug_struct("ReplyVia").field("set", &set).finish()
    }
}

/// Socket facts that travel with `ServerEvent::PeerHandshake`.
#[derive(Clone)]
pub struct HandshakeRoute {
    /// The socket the request arrived on (JS `req.socket`): a direct connection
    /// is answered and dialled from it, whatever the node's firewall state.
    pub arrived_on: UdxSocket,
    /// See [`ReplyVia`].
    pub reply_via: ReplyVia,
}

impl HandshakeRoute {
    pub(crate) fn new(arrived_on: UdxSocket) -> Self {
        Self {
            arrived_on,
            reply_via: ReplyVia::default(),
        }
    }
}

impl fmt::Debug for HandshakeRoute {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HandshakeRoute")
            .field("reply_via", &self.reply_via)
            .finish_non_exhaustive()
    }
}

/// Where the server's relayed answer (`FROM_SERVER`) goes: `FROM_RELAY` back to the
/// node that forwarded it, `FROM_SECOND_RELAY` to the first relay named in the message
/// (`None` for a missing `relay_address`: JS stays silent).
pub(crate) fn relay_target(
    mode: u64,
    from: &Ipv4Peer,
    relay_address: Option<&Ipv4Peer>,
) -> Option<Ipv4Peer> {
    match mode {
        MODE_FROM_SECOND_RELAY => relay_address.cloned(),
        MODE_FROM_RELAY => Some(from.clone()),
        _ => None,
    }
}

#[cfg(test)]
#[path = "handshake_route_tests.rs"]
mod tests;
