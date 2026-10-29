// JS: `dht.createServer` from mock-dht.cjs (`server.listen`, `firewall.probe`, a fake connection via MOCK_FAKE_CONN)
// and hyperdht/lib/server.js (accepting incoming connections). Related: dht.rs creates servers, shared.rs stores the registry.
use crate::lock::lock;
use crate::mock::dht::NodeState;
use crate::mock::events;
use crate::mock::shared::Shared;
use crate::mock::stub::StubStream;
use std::fmt;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::Sender;
use townjump_dht::{DhtConnection, DhtError, DhtServer, Firewall, KeyPair};

/// Key the server tries in the firewall besides its own (`Buffer.alloc(32, 7)` in mock-dht.cjs).
const OTHER_KEY: [u8; 32] = [7; 32];
/// Key of the fake incoming connection (`Buffer.alloc(32, 9)`).
const FAKE_PEER: [u8; 32] = [9; 32];

pub(crate) fn refused() -> DhtError {
    DhtError::ConnectionFailed("Could not connect to peer".to_string())
}

pub(crate) struct ServerInner {
    shared: Arc<Shared>,
    node: Arc<NodeState>,
    firewall: Option<Firewall>,
    relayed: bool,
    tx: Mutex<Option<Sender<DhtConnection>>>,
    listening: Mutex<Option<[u8; 32]>>,
}

impl ServerInner {
    /// Whether the server lets in a client with identity `identity`: the node is not suspended or destroyed, the firewall did not reject.
    pub(crate) fn admit(&self, identity: &[u8; 32]) -> Result<(), DhtError> {
        let denied = self.firewall.as_ref().is_some_and(|deny| deny(identity));
        let unavailable = self.node.is_suspended() || self.node.is_destroyed();
        if denied || unavailable {
            return Err(refused());
        }
        Ok(())
    }

    /// Whether the server was created with `relay_through` (its incoming connections carry the relayed label).
    pub(crate) fn is_relayed(&self) -> bool {
        self.relayed
    }

    /// Puts a connection into the incoming queue; a closed server or a dropped receiver is a rejection.
    pub(crate) async fn deliver(&self, conn: DhtConnection) -> Result<(), DhtError> {
        let tx = lock(&self.tx).clone().ok_or_else(refused)?;
        tx.send(conn).await.map_err(|_| refused())
    }

    fn check_can_listen(&self) -> Result<(), DhtError> {
        if self.node.is_destroyed() {
            return Err(DhtError::Destroyed);
        }
        if lock(&self.listening).is_some() {
            return Err(DhtError::Backend("Server already listening".to_string()));
        }
        if lock(&self.tx).is_none() {
            return Err(DhtError::Backend("Server closed".to_string()));
        }
        Ok(())
    }

    /// `firewall.probe`: the callback gets the own key, then `[7; 32]`; without a firewall there is no event.
    fn probe(&self, own: &[u8; 32]) {
        let Some(deny) = &self.firewall else { return };
        let (own_key, other_key) = (deny(own), deny(&OTHER_KEY));
        self.shared.emit(events::probe_event(own_key, other_key));
    }

    /// `MOCK_FAKE_CONN`: right after `listen` a connection from `[9; 32]` is placed in the incoming queue.
    fn offer_fake_connection(&self) {
        if !self.shared.config.fake_conn {
            return;
        }
        let Some(tx) = lock(&self.tx).clone() else {
            return;
        };
        let stream = Box::new(StubStream);
        let _ = tx.try_send(DhtConnection::new(FAKE_PEER, stream).silent_close());
    }
}

/// Mock server; clones are the same server.
#[derive(Clone)]
pub struct MockServer(Arc<ServerInner>);

impl fmt::Debug for MockServer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MockServer").finish_non_exhaustive()
    }
}

impl MockServer {
    pub(crate) fn new(
        shared: Arc<Shared>,
        node: Arc<NodeState>,
        firewall: Option<Firewall>,
        relayed: bool,
        tx: Sender<DhtConnection>,
    ) -> Self {
        MockServer(Arc::new(ServerInner {
            shared,
            node,
            firewall,
            relayed,
            tx: Mutex::new(Some(tx)),
            listening: Mutex::new(None),
        }))
    }

    /// Closes the server: incoming connections end, the key is released. Idempotent, writes no events.
    pub(crate) fn shut(&self) {
        let inner = &self.0;
        lock(&inner.tx).take();
        let Some(pk) = lock(&inner.listening).take() else {
            return;
        };
        inner.shared.unregister(&pk, inner);
    }

    fn start(&self, kp: &KeyPair) -> Result<(), DhtError> {
        let inner = &self.0;
        inner.shared.emit(events::listen_event(&kp.public_key));
        inner.check_can_listen()?;
        inner.probe(&kp.public_key);
        *lock(&inner.listening) = Some(kp.public_key);
        inner.shared.register(kp.public_key, inner.clone());
        inner.offer_fake_connection();
        Ok(())
    }
}

impl DhtServer for MockServer {
    async fn listen(&self, kp: KeyPair) -> Result<(), DhtError> {
        self.0.shared.switches.before_listen().await?;
        self.start(&kp)
    }

    async fn close(&self) -> Result<(), DhtError> {
        self.shut();
        Ok(())
    }
}

#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;
