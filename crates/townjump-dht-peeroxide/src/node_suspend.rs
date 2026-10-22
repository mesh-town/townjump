// JS: `HyperDHT.suspend/resume` (hyperdht/index.js): `rawStreams.clear()` before and after `super.suspend()`.
// See also: tunnels.rs (tunnel registry), node.rs (node state), dht.rs (the `Dht` trait calls from here).
use crate::error::{map_error, suspended};
use crate::node::Node;
use peeroxide_dht::hyperdht::PeerConnection;
use townjump_dht::{DhtConnection, DhtError};

impl Node {
    /// The fork `suspend()` leaves connections alone, JS tears them all down: the adapter closes its own tunnels.
    /// The second pass catches handshakes that arrived during `suspend` (like the second `rawStreams.clear()`).
    /// A destroyed node — `Ok`, like JS.
    pub async fn suspend(&self) -> Result<(), DhtError> {
        if self.check_alive().is_err() {
            return Ok(());
        }
        self.tunnels.close_all();
        let done = self.handle.suspend().await.map_err(map_error);
        self.tunnels.close_all();
        done
    }

    /// Resumes the node and servers; torn-down tunnels do not revive, new ones are opened afresh.
    pub async fn resume(&self) -> Result<(), DhtError> {
        if self.check_alive().is_err() {
            return Ok(());
        }
        self.handle.resume().await.map_err(map_error)
    }

    /// Fork connection → consumer. The slot is recorded before the check: a node that is suspended later will tear it down with the second
    /// `suspend` pass; an already suspended one — the connection is dropped here (JS `SUSPENDED` for an outgoing one).
    pub fn adopt(&self, conn: PeerConnection) -> Result<DhtConnection, DhtError> {
        let wrapped = self.tunnels.register(conn);
        match self.handle.is_suspended() {
            true => Err(suspended()),
            false => Ok(wrapped),
        }
    }
}

#[cfg(test)]
#[path = "node_suspend_tests.rs"]
mod tests;
