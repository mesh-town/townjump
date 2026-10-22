// JS: hyperdht/lib/connect.js:findAndConnect (end), connectThroughNode
//! Which error `connect_with_options` returns when every attempt failed. Neighbours: `connect_flow.rs`, `connect_race.rs`.
//!
//! JS swallows the failures of the handshake stage (timeout, `BAD_HANDSHAKE_REPLY`, unreadable
//! Noise) and ends with `attempts ? PEER_CONNECTION_FAILED : PEER_NOT_FOUND`, where `attempts`
//! counts the holders of the peer record (FIND_PEER replies with a value). Failures after the
//! reply was accepted (`SERVER_ERROR`, holepunch) destroy the socket at once and are kept.

use super::HyperDhtError;
use crate::rpc::DhtError;

fn swallowed_by_js(e: &HyperDhtError) -> bool {
    match e {
        HyperDhtError::NoRelayNodes
        | HyperDhtError::HandshakeFailed(_)
        | HyperDhtError::Router(_)
        | HyperDhtError::Noise(_)
        | HyperDhtError::NoiseWrap(_)
        | HyperDhtError::Encoding(_) => true,
        HyperDhtError::Dht(rpc) => !matches!(rpc, DhtError::Destroyed | DhtError::ChannelClosed),
        _ => false,
    }
}

/// A failure JS does not swallow: it destroys the connection at once, no other holder is tried.
pub(super) fn is_terminal(e: &HyperDhtError) -> bool {
    !swallowed_by_js(e)
}

/// The error to remember after a failed attempt: a swallowed one never replaces a kept one.
pub(super) fn keep_terminal(current: HyperDhtError, new: HyperDhtError) -> HyperDhtError {
    if swallowed_by_js(&new) && !swallowed_by_js(&current) {
        return current;
    }
    new
}

/// The final error: a kept one as is, else "holders were tried" or "nobody holds the record".
pub(super) fn connect_failure(holders: usize, last: HyperDhtError) -> HyperDhtError {
    if !swallowed_by_js(&last) {
        return last;
    }
    if holders > 0 {
        return HyperDhtError::NoRelayNodes;
    }
    HyperDhtError::PeerNotFound
}

#[cfg(test)]
#[path = "connect_error_tests.rs"]
mod tests;
