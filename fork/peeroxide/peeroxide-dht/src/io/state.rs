// JS: dht-rpc/lib/io.js
//! `struct Io` and its constructor/teardown/socket accessors.
//! Neighbours: `event_loop`, `requests`, `replies`, `tokens`, `relay`, `datagram` extend `impl Io`.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use libudx::{Datagram, UdxRuntime, UdxSocket};

use crate::routing_table::RoutingTable;

use super::types::{
    InflightEntry, IoConfig, IoError, IoResult, IoStats, PendingSend, SocketKind, WireCounters,
};
use super::window::CongestionWindow;

// ── Io ────────────────────────────────────────────────────────────────────────

pub struct Io {
    pub(super) client_socket: UdxSocket,
    pub(super) server_socket: UdxSocket,
    pub(super) client_rx: tokio::sync::mpsc::UnboundedReceiver<Datagram>,
    pub(super) server_rx: tokio::sync::mpsc::UnboundedReceiver<Datagram>,
    pub(super) inflight: Vec<InflightEntry>,
    pub(super) congestion: CongestionWindow,
    pub(super) pending: VecDeque<PendingSend>,
    pub(super) tid: u16,
    pub(super) secrets: Option<[[u8; 32]; 2]>,
    pub(super) rotate_countdown: u32,
    pub(super) firewalled: bool,
    pub ephemeral: bool,
    pub stats: IoStats,
    pub wire: WireCounters,
    pub(super) table: Arc<Mutex<RoutingTable>>,
    pub(super) destroying: bool,
    pub(super) suspended: bool,
}

impl Io {
    /// Create and bind the IO layer (two sockets).
    pub async fn bind(
        runtime: &UdxRuntime,
        table: Arc<Mutex<RoutingTable>>,
        config: IoConfig,
    ) -> IoResult<Self> {
        let server_addr: SocketAddr = format!("{}:{}", config.host, config.port)
            .parse()
            .map_err(IoError::AddrParse)?;
        let client_addr: SocketAddr = format!("{}:0", config.host)
            .parse()
            .map_err(IoError::AddrParse)?;

        let server_socket = runtime.create_socket().await?;
        server_socket.bind(server_addr).await?;
        let server_rx = server_socket.recv_start()?;

        let client_socket = runtime.create_socket().await?;
        client_socket.bind(client_addr).await?;
        let client_rx = client_socket.recv_start()?;

        let tid: u16 = rand::random();

        Ok(Io {
            client_socket,
            server_socket,
            client_rx,
            server_rx,
            inflight: Vec::new(),
            congestion: CongestionWindow::new(config.max_window),
            pending: VecDeque::new(),
            tid,
            secrets: None,
            rotate_countdown: 10,
            firewalled: config.firewalled,
            ephemeral: config.ephemeral,
            stats: IoStats::default(),
            wire: WireCounters::default(),
            table,
            destroying: false,
            suspended: false,
        })
    }

    pub(crate) fn set_firewalled(&mut self, on: bool) {
        self.firewalled = on;
    }

    pub(crate) fn is_firewalled(&self) -> bool {
        self.firewalled
    }

    /// Get a clone of the wire-byte counters. Cheap (Arc clone).
    pub fn wire_counters(&self) -> WireCounters {
        self.wire.clone()
    }

    pub async fn server_local_addr(&self) -> IoResult<std::net::SocketAddr> {
        self.server_socket.local_addr().await.map_err(IoError::from)
    }

    pub fn server_socket(&self) -> UdxSocket {
        self.server_socket.clone()
    }

    /// The socket that carries messages of `kind` (the one a request arrived on).
    pub(crate) fn socket_for(&self, kind: SocketKind) -> UdxSocket {
        match kind {
            SocketKind::Client => self.client_socket.clone(),
            SocketKind::Server => self.server_socket.clone(),
        }
    }

    pub fn primary_socket(&self) -> UdxSocket {
        if self.firewalled {
            self.client_socket.clone()
        } else {
            self.server_socket.clone()
        }
    }

    /// Destroy the IO layer, closing both sockets.
    pub async fn destroy(mut self) -> IoResult<()> {
        self.destroying = true;
        for entry in self.inflight.drain(..) {
            self.congestion.recv();
            self.stats.active = self.stats.active.saturating_sub(1);
            tracing::debug!(tid = entry.tid, "destroy: dropping inflight request");
        }
        self.client_socket.close().await?;
        self.server_socket.close().await?;
        Ok(())
    }
}
