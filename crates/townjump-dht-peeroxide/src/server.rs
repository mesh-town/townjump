// JS: `server.listen(keyPair)` and `server.close()` (hyperdht/lib/server.js).
// See also: dht.rs (`create_server`), node.rs (node state), stream.rs (incoming wrapper).
use crate::error::map_error;
use crate::node::{Lan, Node, fork_pair};
use crate::opts_map::fork_opts;
use peeroxide_dht::hyperdht::PeerConnection;
use peeroxide_dht::server::Server;
use std::fmt;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use townjump_dht::{DhtConnection, DhtError, DhtServer, Incoming, ServerOpts};
use townjump_keys::KeyPair;

/// Incoming queue held by the consumer: like `server.on('connection')`, lossless under slow reads.
const INCOMING_CAPACITY: usize = 16;

/// DHT node server.
#[derive(Clone)]
pub struct PeeroxideServer {
    shared: Arc<Shared>,
    node: Arc<Node>,
}

/// Shared by server clones and the node: the fork server and the task forwarding incoming connections to the consumer.
pub(crate) struct Shared {
    server: Server,
    forward: Mutex<Option<JoinHandle<()>>>,
    /// LAN target (hash of the public key) while the server listens.
    lan: Arc<Lan>,
    lan_target: Mutex<Option<[u8; 32]>>,
}

impl PeeroxideServer {
    /// Fork server on node events and the task that hands incoming connections to the consumer as `DhtConnection`.
    pub(crate) fn create(
        node: Arc<Node>,
        opts: ServerOpts,
    ) -> Result<(PeeroxideServer, Incoming), DhtError> {
        let events = node.take_events()?;
        let runtime = libudx::UdxRuntime::shared(node.runtime.handle());
        let (server, from_fork) = Server::new(
            node.handle.clone(),
            events,
            runtime,
            fork_opts(opts, node.identity.clone()),
        );
        let (sender, incoming) = Incoming::channel(INCOMING_CAPACITY);
        let forward = tokio::spawn(forward(from_fork, sender, Arc::clone(&node)));
        let shared = Arc::new(Shared {
            server,
            forward: Mutex::new(Some(forward)),
            lan: Arc::clone(&node.lan),
            lan_target: Mutex::new(None),
        });
        node.track(&shared);
        Ok((PeeroxideServer { shared, node }, incoming))
    }
}

impl Shared {
    /// Withdraws the announce and aborts forwarding: the fork channel is not closed after `close`, yet `Incoming` must end.
    pub(crate) async fn close(&self) -> Result<(), DhtError> {
        let target = self
            .lan_target
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(target) = target {
            self.lan.unregister(&target);
        }
        let closed = self.server.close().await.map_err(map_error);
        let forward = self
            .forward
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        forward.inspect(|task| task.abort());
        closed
    }
}

impl fmt::Debug for PeeroxideServer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PeeroxideServer").finish_non_exhaustive()
    }
}

impl PeeroxideServer {
    /// After the fork accepts handshakes (before that a direct request is answered without noise): LAN registration of
    /// the hashed key at the IO port of the node.
    async fn announce_on_lan(&self, public_key: [u8; 32]) {
        let port = match self.node.handle.local_port().await {
            Ok(port) => port,
            Err(e) => return tracing::debug!("lan: no local port: {e}"),
        };
        let target = peeroxide_dht::crypto::hash(&public_key);
        *self
            .shared
            .lan_target
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(target);
        self.shared.lan.register(target, port);
    }
}

impl DhtServer for PeeroxideServer {
    /// JS: `NODE_DESTROYED` on a destroyed node. Waits only for the node being alive, like `Server.listen` in JS
    /// (`dht.bind()`); the announce tops its query up from the bootstrap nodes itself.
    async fn listen(&self, kp: KeyPair) -> Result<(), DhtError> {
        self.node.check_alive()?;
        self.shared
            .server
            .listen(fork_pair(&kp))
            .await
            .map_err(map_error)?;
        self.announce_on_lan(kp.public_key).await;
        Ok(())
    }

    async fn close(&self) -> Result<(), DhtError> {
        self.shared.close().await
    }
}

/// Fork incoming → consumer via `Node::adopt` (tunnel registry, keepalive); ends together with the server
/// (fork channel closes) or the consumer. A connection that arrives at a sleeping node is dropped silently.
async fn forward(
    mut from_fork: mpsc::Receiver<PeerConnection>,
    to_consumer: mpsc::Sender<DhtConnection>,
    node: Arc<Node>,
) {
    while let Some(conn) = from_fork.recv().await {
        let Ok(adopted) = node.adopt(conn) else {
            continue;
        };
        if to_consumer.send(adopted).await.is_err() {
            return;
        }
    }
}

#[cfg(test)]
#[path = "server_listen_tests.rs"]
mod listen_tests;
#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;
