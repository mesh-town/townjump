// JS: hyperdht/lib/errors.js:DHTError (codes PEER_NOT_FOUND, PEER_CONNECTION_FAILED, …).
// See also: dht.rs, server.rs, stream.rs call `map_error`; `From` is impossible here (both types are foreign).
use libudx::UdxError;
use peeroxide_dht::hyperdht::HyperDhtError;
use peeroxide_dht::rpc::DhtError as RpcError;
use townjump_dht::DhtError;

/// JS default text of `PEER_CONNECTION_FAILED`.
const CONNECT_FAILED: &str = "Could not connect to peer";

/// Fork error → `DhtError`, the hyperdht code is kept in the text.
///
/// A closed command channel means a stopped node (the actor exits only on `destroy`), so it
/// reads as `NODE_DESTROYED`, like any operation rejected after `dht.destroy()` in JS.
pub fn map_error(e: HyperDhtError) -> DhtError {
    match e {
        HyperDhtError::Destroyed | HyperDhtError::ChannelClosed => DhtError::Destroyed,
        HyperDhtError::Dht(rpc) => map_rpc(rpc),
        HyperDhtError::PeerNotFound => DhtError::PeerNotFound,
        HyperDhtError::HolepunchAborted => DhtError::HolepunchAborted,
        HyperDhtError::NoRelayNodes | HyperDhtError::FirewallRejected => connection_failed(),
        HyperDhtError::StreamEstablishment(text) => DhtError::ConnectionFailed(text),
        HyperDhtError::HandshakeFailed(text) => coded("BAD_HANDSHAKE_REPLY", &text),
        HyperDhtError::HolepunchFailed => coded("CANNOT_HOLEPUNCH", "Cannot holepunch to remote"),
        HyperDhtError::Udx(UdxError::Io(io)) => DhtError::Io(io),
        HyperDhtError::RelayAborted => coded("RELAY_ABORTED", "Relay aborted"),
        HyperDhtError::AlreadyListening => coded("ALREADY_LISTENING", "Already listening"),
        HyperDhtError::ServerClosed => DhtError::Backend("Server closed".to_string()),
        other => DhtError::Backend(other.to_string()),
    }
}

fn map_rpc(e: RpcError) -> DhtError {
    match e {
        RpcError::Destroyed | RpcError::ChannelClosed => DhtError::Destroyed,
        RpcError::Suspended => suspended(),
        other => DhtError::Backend(other.to_string()),
    }
}

/// JS `SUSPENDED` (hyperdht/lib/errors.js): an outgoing action of a suspended node.
pub fn suspended() -> DhtError {
    coded("SUSPENDED", "Suspended")
}

fn connection_failed() -> DhtError {
    DhtError::ConnectionFailed(CONNECT_FAILED.to_string())
}

fn coded(code: &str, text: &str) -> DhtError {
    DhtError::Backend(format!("{code}: {text}"))
}

#[cfg(test)]
#[path = "error_tests.rs"]
mod tests;
