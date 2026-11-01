// JS: `HyperDHT` state (defaultKeyPair, destroyed, ready, listening) — shared by the node and its servers.
// See also: dht.rs (the `Dht` trait), server.rs (the server holds an `Arc<Node>`), factory.rs (`Node::start`).
use crate::error::map_error;
use crate::lan_race::{PROBE_START, ProbeStart};
use crate::server::Shared;
use crate::tunnels::Tunnels;
use libudx::UdxRuntime;
use peeroxide_dht::hyperdht::{self, HyperDhtConfig, HyperDhtError, HyperDhtHandle, ServerEvent};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::Duration;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::task::JoinHandle;
use townjump_dht::DhtError;
use townjump_keys::KeyPair;
use townjump_lan::{LanConfig, ProbeTiming, Responder, Target};

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
    /// LAN discovery: the responder and the probe settings (lan_race.rs).
    pub lan: Arc<Lan>,
}

/// LAN discovery settings of a node; `enabled: false` — no responder and no probe.
#[derive(Clone, Debug)]
pub(crate) struct LanSetup {
    pub enabled: bool,
    pub config: LanConfig,
    pub timing: ProbeTiming,
    pub start: ProbeStart,
}

impl Default for LanSetup {
    fn default() -> Self {
        LanSetup {
            enabled: true,
            config: LanConfig::default(),
            timing: ProbeTiming::default(),
            start: PROBE_START,
        }
    }
}

/// The node's LAN responder: starts on the first registration, silent while the node sleeps, gone after `close`.
pub(crate) struct Lan {
    pub setup: LanSetup,
    state: Mutex<LanState>,
}

#[derive(Default)]
struct LanState {
    responder: Option<Responder>,
    paused: bool,
    closed: bool,
}

impl Lan {
    fn new(setup: LanSetup) -> Lan {
        Lan {
            setup,
            state: Mutex::new(LanState::default()),
        }
    }

    /// Answers requests for `target` with `port`. A start failure (no rights, port) is a `debug` line: the node lives without LAN.
    pub fn register(&self, target: Target, port: u16) {
        let mut state = lock(&self.state);
        if !self.setup.enabled || state.closed {
            return;
        }
        if state.responder.is_none() {
            match Responder::start(&self.setup.config) {
                Ok(responder) => state.responder = Some(responder),
                Err(e) => return tracing::debug!("lan: responder did not start: {e}"),
            }
        }
        let paused = state.paused;
        if let Some(responder) = &state.responder {
            responder.set_paused(paused);
            responder.register(target, port);
        }
    }

    pub fn unregister(&self, target: &Target) {
        if let Some(responder) = &lock(&self.state).responder {
            responder.unregister(target);
        }
    }

    pub fn set_paused(&self, paused: bool) {
        let mut state = lock(&self.state);
        state.paused = paused;
        if let Some(responder) = &state.responder {
            responder.set_paused(paused);
        }
    }

    pub fn close(&self) {
        let mut state = lock(&self.state);
        state.closed = true;
        if let Some(responder) = state.responder.take() {
            responder.close();
        }
    }
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
        Self::start_with(config, identity, LanSetup::default()).await
    }

    pub async fn start_with(
        config: HyperDhtConfig,
        identity: Option<KeyPair>,
        lan: LanSetup,
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
            lan: Arc::new(Lan::new(lan)),
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

    /// Routing table size (LAN probe trigger, lan_race.rs); a failed request counts as an empty table.
    pub async fn table_size(&self) -> usize {
        self.handle.table_size().await.unwrap_or(0)
    }

    /// Bootstrap finished — without waiting for it (`bootstrapped()` replies at once when it is done).
    pub async fn bootstrap_done(&self) -> bool {
        let wait = self.handle.bootstrapped();
        tokio::time::timeout(Duration::from_millis(20), wait)
            .await
            .is_ok_and(|r| r.is_ok())
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
        self.lan.close();
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
