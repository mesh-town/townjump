// JS: `HyperDHT` state (defaultKeyPair, destroyed, ready, listening) — shared by the node and its servers.
// See also: dht.rs (the `Dht` trait), server.rs (the server holds an `Arc<Node>`), factory.rs (`Node::start`).
use crate::error::map_error;
use crate::server::Shared;
use crate::tunnels::Tunnels;
use libudx::UdxRuntime;
use peeroxide_dht::hyperdht::{self, HyperDhtConfig, HyperDhtError, HyperDhtHandle, ServerEvent};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::task::JoinHandle;
use townjump_dht::DhtError;
use townjump_keys::KeyPair;

type Events = UnboundedReceiver<ServerEvent>;

/// Fork node: handle, UDX runtime (one per node), client identity and server accounting.
pub(crate) struct Node {
    pub handle: HyperDhtHandle,
    pub runtime: UdxRuntime,
    pub identity: hyperdht::KeyPair,
    /// Streams issued upward: `suspend` tears them down (node_suspend.rs).
    pub tunnels: Tunnels,
    events: Mutex<Option<Events>>,
    servers: Mutex<Vec<Weak<Shared>>>,
    task: Mutex<Option<JoinHandle<Result<(), HyperDhtError>>>>,
    destroyed: AtomicBool,
}

/// holesail key pair → fork pair (same format: secret = seed ‖ public).
pub(crate) fn fork_pair(kp: &KeyPair) -> hyperdht::KeyPair {
    hyperdht::KeyPair {
        public_key: kp.public_key,
        secret_key: kp.secret_key,
    }
}

impl Node {
    /// Starts the node; `identity: None` — a random pair, like `HyperDHT.keyPair()`. Does not wait for bootstrap (like JS).
    pub async fn start(
        config: HyperDhtConfig,
        identity: Option<KeyPair>,
    ) -> Result<Arc<Node>, DhtError> {
        let runtime = UdxRuntime::new().map_err(|e| map_error(e.into()))?;
        let (task, handle, events) = hyperdht::spawn(&runtime, config).await.map_err(map_error)?;
        let identity = identity
            .as_ref()
            .map_or_else(hyperdht::KeyPair::generate, fork_pair);
        Ok(Arc::new(Node {
            handle,
            runtime,
            identity,
            tunnels: Tunnels::default(),
            events: Mutex::new(Some(events)),
            servers: Mutex::new(Vec::new()),
            task: Mutex::new(Some(task)),
            destroyed: AtomicBool::new(false),
        }))
    }

    /// `NODE_DESTROYED` if the node is already destroyed.
    pub fn check_alive(&self) -> Result<(), DhtError> {
        match self.destroyed.load(Ordering::SeqCst) {
            true => Err(DhtError::Destroyed),
            false => Ok(()),
        }
    }

    /// A live node that finished bootstrap (JS: `await dht.ready()` inside operations).
    pub async fn ready(&self) -> Result<(), DhtError> {
        self.check_alive()?;
        self.handle.bootstrapped().await.map_err(map_error)
    }

    /// The `spawn` event channel is handed to the node's only server.
    pub fn take_events(&self) -> Result<Events, DhtError> {
        let taken = lock(&self.events).take();
        taken.ok_or_else(|| DhtError::Backend("Only one server per node is supported".to_string()))
    }

    pub fn track(&self, server: &Arc<Shared>) {
        lock(&self.servers).push(Arc::downgrade(server));
    }

    /// Closes the node's servers (like `dht.destroy()` in JS), then the node; a repeated call is `Ok`.
    pub async fn destroy(&self) -> Result<(), DhtError> {
        if self.destroyed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let servers: Vec<Arc<Shared>> = lock(&self.servers)
            .drain(..)
            .filter_map(|w| w.upgrade())
            .collect();
        for server in servers {
            let _ = server.close().await;
        }
        let destroyed = self.handle.destroy().await.map_err(map_error);
        lock(&self.task).take().inspect(|task| task.abort());
        destroyed
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}
