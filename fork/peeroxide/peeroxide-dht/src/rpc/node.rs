// JS: dht-rpc/index.js
//! `struct DhtNode` (the background actor), its `run` loop, `start_bootstrap` and `spawn`.
//! Neighbours: `node_events`, `node_responses`, `node_tick`, `node_command`, `node_query`, `node_table`
//! extend `impl DhtNode`.

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{mpsc, oneshot, watch};
use tokio::time::{Instant, interval};

use libudx::UdxRuntime;

use crate::io::{Io, IoConfig};
use crate::messages::Ipv4Peer;
use crate::peer::{NodeId, peer_id};
use crate::query::{Query, QueryResult, parse_bootstrap_str, resolve_bootstrap_nodes};
use crate::routing_table::RoutingTable;

use super::adaptive::Adaptive;
use super::command::DhtCommand;
use super::consts::{CMD_FIND_NODE, DRAIN_INTERVAL_MS, REFRESH_TICKS, TICK_INTERVAL_MS};
use super::handle::DhtHandle;
use super::nat_check::NatCheck;
use super::node_adaptive::{adaptive_on, joins_network, node_address, start_mode};
use super::node_nat::NodeNat;
use super::requests::{DeferredReply, StandaloneRequest, UserRequest};
use super::types::{DhtConfig, DhtError, DhtMode};

// ── DhtNode (internal background actor) ──────────────────────────────────────

pub(super) struct DhtNode {
    pub(super) io: Io,
    pub(super) table: Arc<Mutex<RoutingTable>>,
    pub(super) config: DhtConfig,
    pub(super) local_port: u16,
    pub(super) tick: u64,
    pub(super) refresh_ticks: u64,
    pub(super) repinging: u32,
    pub(super) bootstrapped: bool,
    pub(super) bootstrap_waiters: Vec<oneshot::Sender<Result<(), DhtError>>>,

    pub(super) active_queries: HashMap<u64, Query>,
    pub(super) tid_to_query: HashMap<u16, (u64, bool)>,
    pub(super) standalone_tids: HashMap<u16, StandaloneRequest>,
    pub(super) next_query_id: u64,

    pub(super) cmd_rx: mpsc::UnboundedReceiver<DhtCommand>,
    pub(super) request_subscribers: Vec<mpsc::UnboundedSender<UserRequest>>,

    pub(super) destroyed: bool,
    pub(super) suspended: Arc<AtomicBool>,
    pub(super) last_tick_time: Instant,
    pub(super) down_hints_per_tick: u32,
    pub(super) bootstrap_query_id: Option<u64>,
    pub(super) deferred_reply_tx: mpsc::UnboundedSender<DeferredReply>,
    pub(super) deferred_reply_rx: mpsc::UnboundedReceiver<DeferredReply>,

    pub(super) needs_id_update: bool,
    pub(super) addr_samples: Vec<Ipv4Peer>,
    pub(super) node_nat: NodeNat,

    pub(super) adaptive: Adaptive,
    pub(super) nat_check: Option<NatCheck>,
    /// host:port the table id is derived from while the node is persistent.
    pub(super) node_address: Option<Ipv4Peer>,
    pub(super) mode_tx: watch::Sender<DhtMode>,
}

impl DhtNode {
    async fn run(mut self) -> Result<(), DhtError> {
        let own_id = {
            let t = self.table.lock().map_err(|_| DhtError::ChannelClosed)?;
            *t.id()
        };

        self.add_seed_nodes();
        self.start_bootstrap(own_id);

        let mut drain_interval = interval(Duration::from_millis(DRAIN_INTERVAL_MS));
        let mut tick_interval = interval(Duration::from_millis(TICK_INTERVAL_MS));

        loop {
            if self.destroyed {
                break;
            }

            let timeout_at = self.io.next_timeout_deadline();

            tokio::select! {
                biased;

                Some(event) = self.io.recv() => {
                    self.handle_io_event(event);
                },

                _ = drain_interval.tick() => {
                    self.io.drain();
                },

                _ = tokio::time::sleep_until(timeout_at) => {
                    self.handle_expired();
                },

                _ = tick_interval.tick() => {
                    self.handle_tick();
                },

                Some(reply) = self.deferred_reply_rx.recv() => {
                    self.handle_deferred_reply(reply);
                },

                Some(cmd) = self.cmd_rx.recv() => {
                    if self.handle_command(cmd) {
                        break;
                    }
                },
            }
        }

        self.io.destroy().await?;
        Ok(())
    }

    pub(super) fn start_bootstrap(&mut self, own_id: NodeId) {
        let bootstrap_nodes: Vec<(String, u16)> = self
            .config
            .bootstrap
            .iter()
            .filter_map(|s| parse_bootstrap_str(s))
            .collect();

        let (result_tx, _result_rx) = oneshot::channel::<QueryResult>();
        let query_id = self.next_query_id;
        self.next_query_id += 1;
        self.bootstrap_query_id = Some(query_id);

        let mut q = Query::new(
            query_id,
            own_id,
            true,
            CMD_FIND_NODE,
            None,
            self.config.concurrency,
            false,
            result_tx,
            Arc::clone(&self.table),
            own_id,
        );

        q.add_from_table();
        q.add_nodes(&bootstrap_nodes);

        let requests = q.poll_requests();
        self.dispatch_query_requests(query_id, requests);

        if q.is_finished() {
            self.on_query_removed(query_id);
        } else {
            self.active_queries.insert(query_id, q);
        }

        tracing::debug!(query_id, "bootstrap query started");
    }
}

// ── Spawn ─────────────────────────────────────────────────────────────────────

/// Spawns a DHT node and returns its join handle and public handle.
pub async fn spawn(
    runtime: &UdxRuntime,
    config: DhtConfig,
) -> Result<(tokio::task::JoinHandle<Result<(), DhtError>>, DhtHandle), DhtError> {
    let (node, dht_handle) = build_node(runtime, config).await?;
    let handle = tokio::spawn(node.run());
    Ok((handle, dht_handle))
}

/// Builds the node and the public handle without starting the loop (`spawn` = this + `tokio::spawn`).
pub(super) async fn build_node(
    runtime: &UdxRuntime,
    config: DhtConfig,
) -> Result<(DhtNode, DhtHandle), DhtError> {
    let table_id: NodeId = rand::random();
    let table = Arc::new(Mutex::new(RoutingTable::new(table_id)));
    let table_for_handle = Arc::clone(&table);

    // Adaptive nodes start ephemeral too: bootstrap or seed nodes already mean that.
    let adaptive = Adaptive::new(adaptive_on(&config), config.stable_ticks);
    let ephemeral = config.ephemeral.unwrap_or(joins_network(&config));
    let io_config = IoConfig {
        max_window: config.max_window,
        port: config.port,
        host: config.host.clone(),
        firewalled: config.firewalled,
        ephemeral,
    };

    let io = Io::bind(runtime, Arc::clone(&table), io_config).await?;
    let local_port = io
        .server_local_addr()
        .await
        .map(|a| a.port())
        .unwrap_or(config.port);

    let is_wildcard = config.host == "0.0.0.0" || config.host == "::";
    let needs_id_update = !ephemeral && is_wildcard;

    if !ephemeral && !is_wildcard {
        let deterministic_id = peer_id(&config.host, local_port);
        if let Ok(mut t) = table.lock() {
            t.rebuild_with_id(deterministic_id);
        }
    }

    let node_address = node_address(&config, ephemeral, local_port);
    let (mode_tx, mode) = watch::channel(start_mode(&adaptive, &config, ephemeral, &node_address));

    let mut config = config;
    config.bootstrap = resolve_bootstrap_nodes(&config.bootstrap).await;

    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let (deferred_reply_tx, deferred_reply_rx) = mpsc::unbounded_channel();

    let firewalled = config.firewalled;

    let suspended = Arc::new(AtomicBool::new(false));
    let node = DhtNode {
        io,
        table,
        config,
        local_port,
        tick: 0,
        refresh_ticks: REFRESH_TICKS,
        repinging: 0,
        bootstrapped: false,
        bootstrap_waiters: Vec::new(),
        active_queries: HashMap::new(),
        tid_to_query: HashMap::new(),
        standalone_tids: HashMap::new(),
        next_query_id: 0,
        cmd_rx,
        request_subscribers: Vec::new(),
        destroyed: false,
        suspended: Arc::clone(&suspended),
        last_tick_time: Instant::now(),
        down_hints_per_tick: 0,
        bootstrap_query_id: None,
        deferred_reply_tx,
        deferred_reply_rx,
        needs_id_update,
        addr_samples: Vec::new(),
        node_nat: NodeNat::new(firewalled),
        adaptive,
        nat_check: None,
        node_address,
        mode_tx,
    };

    let wire = node.io.wire_counters();
    let dht_handle = DhtHandle {
        cmd_tx,
        wire,
        table: Arc::clone(&table_for_handle),
        suspended,
        session_id: None,
        mode,
    };

    Ok((node, dht_handle))
}
