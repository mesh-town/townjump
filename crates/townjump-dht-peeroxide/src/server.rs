// JS: `server.listen(keyPair)` and `server.close()` (hyperdht/lib/server.js).
// See also: dht.rs (`create_server`), node.rs (node state), stream.rs (incoming wrapper).
use crate::error::map_error;
use crate::node::{Node, fork_pair};
use peeroxide_dht::hyperdht::{PeerConnection, RelayThrough};
use peeroxide_dht::server::{Firewall as ForkFirewall, Server, ServerOpts as ForkOpts};
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
}

impl PeeroxideServer {
    /// Fork server on node events and the task that hands incoming connections to the consumer as `DhtConnection`.
    pub(crate) fn create(
        node: Arc<Node>,
        opts: ServerOpts,
    ) -> Result<(PeeroxideServer, Incoming), DhtError> {
        let events = node.take_events()?;
        let runtime = libudx::UdxRuntime::shared(node.runtime.handle());
        let (server, from_fork) =
            Server::new(node.handle.clone(), events, runtime, fork_opts(opts));
        let (sender, incoming) = Incoming::channel(INCOMING_CAPACITY);
        let forward = tokio::spawn(forward(from_fork, sender, Arc::clone(&node)));
        let shared = Arc::new(Shared {
            server,
            forward: Mutex::new(Some(forward)),
        });
        node.track(&shared);
        Ok((PeeroxideServer { shared, node }, incoming))
    }
}

impl Shared {
    /// Withdraws the announce and aborts forwarding: the fork channel is not closed after `close`, yet `Incoming` must end.
    pub(crate) async fn close(&self) -> Result<(), DhtError> {
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

impl DhtServer for PeeroxideServer {
    /// JS: `NODE_DESTROYED` on a destroyed node; announce only after bootstrap, otherwise it would go into an empty table.
    async fn listen(&self, kp: KeyPair) -> Result<(), DhtError> {
        self.node.ready().await?;
        self.shared
            .server
            .listen(fork_pair(&kp))
            .await
            .map_err(map_error)
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

/// holesail options → fork options; the callback sees only the client public key.
pub(crate) fn fork_opts(opts: ServerOpts) -> ForkOpts {
    let mut fork = ForkOpts::default();
    fork.firewall = opts
        .firewall
        .map(|f| -> ForkFirewall { Arc::new(move |input| f(input.remote_public_key)) });
    fork.reusable_socket = opts.reusable_socket;
    fork.relay_through = opts.relay_through.map(RelayThrough::Key);
    fork
}

#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;
