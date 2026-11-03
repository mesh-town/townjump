// JS: hyperdht/lib/errors.js:DHTError (`.message` = `<CODE>: <text>`), Node socket errors.
// `Display` is the JS exception's `.message`.
use std::io;

/// DHT network errors.
#[derive(Debug, thiserror::Error)]
pub enum DhtError {
    #[error("NODE_DESTROYED: Node destroyed")]
    Destroyed,
    #[error("PEER_NOT_FOUND: Peer not found")]
    PeerNotFound,
    /// Text after the code (`Could not connect to peer` by default).
    #[error("PEER_CONNECTION_FAILED: {0}")]
    ConnectionFailed(String),
    #[error("HOLEPUNCH_ABORTED: Holepunch aborted")]
    HolepunchAborted,
    #[error(transparent)]
    Io(#[from] io::Error),
    /// Anything else the implementation returned (text as is).
    #[error("{0}")]
    Backend(String),
}

/// JS: dht-rpc/lib/query.js `_endAfterCommit` - the text of a put that no node answered.
pub const NO_NODES_RESPONDED: &str = "Too few nodes responded";

impl DhtError {
    /// True only for `Backend` with exactly the JS text of a put that no node answered.
    pub fn is_no_nodes_responded(&self) -> bool {
        matches!(self, DhtError::Backend(m) if m == NO_NODES_RESPONDED)
    }
}

#[cfg(test)]
#[path = "error_tests.rs"]
mod tests;
