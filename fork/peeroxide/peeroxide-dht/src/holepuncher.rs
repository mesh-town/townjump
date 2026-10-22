//! NAT hole-punching coordination for peer-to-peer connections.
//!
//! The `Holepuncher` drives the active-side hole-punching state machine
//! used by [`crate::hyperdht::HyperDhtHandle::connect`] to traverse symmetric
//! NATs and reach peers that cannot be dialed directly. It owns the local
//! [`crate::socket_pool::SocketPool`] used for birthday-attack probes, the
//! NAT classification analyzer, and the punch-message dispatch loop that
//! cooperates with the remote peer over a relayed control channel.
//!
//! Most consumers reach this module indirectly through `connect()` and the
//! resulting [`crate::hyperdht::PeerConnection`]; direct use is only needed
//! when building custom DHT clients that orchestrate the punch sequence
//! themselves. See `hyperdht/lib/holepuncher.js` in the Node.js reference
//! implementation for the protocol shape.

#![allow(missing_docs)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use libudx::UdxSocket;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{Duration, sleep};

use crate::hyperdht_messages::{FIREWALL_CONSISTENT, FIREWALL_RANDOM, FIREWALL_UNKNOWN};
use crate::messages::Ipv4Peer;
use crate::nat::Nat;
use crate::socket_pool::{
    HolepunchEvent as ProbeEvent, SocketPool, SocketRef, coerce_firewall, random_port,
};

const BIRTHDAY_SOCKETS: usize = 256;

#[non_exhaustive]
pub enum HolepunchEvent {
    Connected { addr: SocketAddr },
    Aborted,
}

pub struct RemoteAddress {
    pub host: String,
    pub port: u16,
    pub verified: bool,
}

pub struct Holepuncher {
    pub(crate) nat: Nat,
    pub is_initiator: bool,
    pub punching: bool,
    pub connected: bool,
    pub destroyed: bool,

    pub remote_firewall: u64,
    pub remote_addresses: Vec<RemoteAddress>,
    pub remote_holepunching: bool,

    sockets: Vec<SocketRef>,
    event_tx: mpsc::UnboundedSender<HolepunchEvent>,

    /// Shared with the recv-adapter task; the initiator branch CAS-flips
    /// this on the first inbound probe so that subsequent probes are
    /// idempotent no-ops. Also consulted by `destroy()` so an Aborted
    /// event isn't emitted on top of a Connected one.
    connected_flag: Arc<AtomicBool>,

    /// Handles to every spawned recv-adapter task: one per socket in
    /// `self.sockets` (the primary socket plus every birthday-attack
    /// socket opened by `open_birthday_sockets`). `destroy()` aborts all
    /// of them so cleanup is bounded and no task outlives the Holepuncher.
    recv_tasks: Vec<JoinHandle<()>>,

    /// Pongs from the background sampler (`start_background_sampling`);
    /// drained into `nat` by `absorb_samples` / `analyze`. Dropping it aborts the pings at once.
    pub(crate) samples: Option<mpsc::UnboundedReceiver<(Ipv4Peer, Ipv4Peer)>>,
}

mod addresses;
mod lifecycle;
mod sampling;

impl Holepuncher {
    pub async fn new(
        pool: &SocketPool,
        runtime: &libudx::UdxRuntime,
        firewalled: bool,
        is_initiator: bool,
        remote_firewall: u64,
        event_tx: mpsc::UnboundedSender<HolepunchEvent>,
    ) -> Result<Self, crate::socket_pool::SocketPoolError> {
        let mut socket = pool.acquire(runtime).await?;

        let connected_flag = Arc::new(AtomicBool::new(false));
        let recv_task = spawn_recv_adapter_for_socket(
            &mut socket,
            is_initiator,
            event_tx.clone(),
            Arc::clone(&connected_flag),
        );

        Ok(Self {
            nat: Nat::new(firewalled),
            is_initiator,
            punching: false,
            connected: false,
            destroyed: false,
            remote_firewall,
            remote_addresses: Vec::new(),
            remote_holepunching: false,
            sockets: vec![socket],
            event_tx,
            connected_flag,
            recv_tasks: recv_task.into_iter().collect(),
            samples: None,
        })
    }

    pub fn update_remote(
        &mut self,
        punching: bool,
        firewall: u64,
        addresses: &[Ipv4Peer],
        verified_host: Option<&str>,
    ) {
        let mut remote_addrs = Vec::new();
        for addr in addresses {
            let is_verified =
                verified_host == Some(addr.host.as_str()) || self.is_verified(&addr.host);
            remote_addrs.push(RemoteAddress {
                host: addr.host.clone(),
                port: addr.port,
                verified: is_verified,
            });
        }
        self.remote_firewall = firewall;
        self.remote_addresses = remote_addrs;
        self.remote_holepunching = punching;
    }

    fn is_verified(&self, host: &str) -> bool {
        self.remote_addresses
            .iter()
            .any(|a| a.verified && a.host == host)
    }

    pub fn primary_socket(&self) -> Option<&SocketRef> {
        self.sockets.first()
    }

    /// Seed `self.nat` with reflexive samples from the puncher socket's
    /// own NAT mapping by sending DHT pings via `dht.ping_via_socket(...)`.
    /// Mirrors Node `lib/holepuncher.js:13-20` + `lib/nat.js:25-79`. Must
    /// run BEFORE [`Self::punch`] or the NAT classification stays
    /// `UNKNOWN` and no punch strategy fires (the
    /// `coerce_firewall(UNKNOWN→CONSISTENT)` MVP lie that this method
    /// replaces; see commit history).
    ///
    /// Server path: starts the reply forwarder itself; the client uses
    /// [`Self::start_background_sampling`] instead.
    ///
    /// Returns the number of new samples added. No-op (returns 0) if the
    /// puncher has no primary socket or its `dht_reply_rx` has already
    /// been consumed.
    pub async fn auto_sample(&mut self, dht: &crate::rpc::DhtHandle) -> usize {
        let (socket_clone, rx) = {
            let Some(socket_ref) = self.sockets.first_mut() else {
                return 0;
            };
            let Some(rx) = socket_ref.dht_reply_rx.take() else {
                return 0;
            };
            (socket_ref.socket.clone(), rx)
        };
        crate::nat_sampler::spawn_dht_reply_forwarder(dht.clone(), rx);
        self.nat
            .auto_sample(dht, socket_clone, crate::nat::NAT_MIN_SAMPLES as usize)
            .await
    }

    fn unstable(&self) -> bool {
        let fw = self.nat.firewall;
        (self.remote_firewall >= FIREWALL_RANDOM && fw >= FIREWALL_RANDOM) || fw == FIREWALL_UNKNOWN
    }

    /// Waits for the background sampler (if any) to settle the NAT, then
    /// decides whether the pair is still unstable.
    pub async fn analyze(&mut self, allow_reopen: bool) -> bool {
        self.wait_samples().await;
        self.nat.analyzing().await;
        if !self.unstable() {
            return true;
        }
        if !allow_reopen {
            return false;
        }
        // Phase 3 MVP divergence from Node `_reopen()`: we do not destroy
        // the NAT and rebuild the socket pool. We give the sampler a
        // brief grace window and re-check `unstable()`. The outer probe
        // loop in `run_holepunch_rounds` provides retry coverage; full
        // reopen is a future extension if real-world peers need it.
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        !self.unstable()
    }

    pub async fn punch(&mut self, pool: &SocketPool, runtime: &libudx::UdxRuntime) -> bool {
        if self.done() || self.remote_addresses.is_empty() {
            return false;
        }

        self.punching = true;

        let local = coerce_firewall(self.nat.firewall);
        let remote = coerce_firewall(self.remote_firewall);

        let verified = self.first_verified_address();

        if local == FIREWALL_CONSISTENT && remote == FIREWALL_CONSISTENT {
            self.consistent_probe().await;
            return true;
        }

        let Some(verified_addr) = verified else {
            return false;
        };

        if local == FIREWALL_CONSISTENT && remote >= FIREWALL_RANDOM {
            self.random_probes(&verified_addr).await;
            return true;
        }

        if local >= FIREWALL_RANDOM && remote == FIREWALL_CONSISTENT {
            self.open_birthday_sockets(pool, runtime, &verified_addr)
                .await;
            if self.punching {
                self.keep_alive_random_nat(&verified_addr).await;
            }
            return true;
        }

        false
    }

    fn first_verified_address(&self) -> Option<Ipv4Peer> {
        self.remote_addresses
            .iter()
            .find(|a| a.verified)
            .map(|a| Ipv4Peer {
                host: a.host.clone(),
                port: a.port,
            })
    }

    async fn consistent_probe(&mut self) {
        if !self.is_initiator {
            sleep(Duration::from_secs(1)).await;
        }

        for tries in 0..10 {
            if !self.punching {
                break;
            }

            for addr in &self.remote_addresses {
                if !addr.verified && (tries & 3) != 0 {
                    continue;
                }
                let target: SocketAddr = match format!("{}:{}", addr.host, addr.port).parse() {
                    Ok(a) => a,
                    Err(_) => continue,
                };
                if let Some(socket) = self.sockets.first() {
                    let _ = socket.send_holepunch(target, false);
                }
            }

            if self.punching {
                sleep(Duration::from_secs(1)).await;
            }
        }

        self.auto_destroy();
    }

    async fn random_probes(&mut self, remote_addr: &Ipv4Peer) {
        for _ in 0..1750 {
            if !self.punching {
                break;
            }

            let port = random_port();
            let target: SocketAddr = match format!("{}:{port}", remote_addr.host).parse() {
                Ok(a) => a,
                Err(_) => continue,
            };
            if let Some(socket) = self.sockets.first() {
                let _ = socket.send_holepunch(target, false);
            }

            if self.punching {
                sleep(Duration::from_millis(20)).await;
            }
        }

        self.auto_destroy();
    }

    async fn open_birthday_sockets(
        &mut self,
        pool: &SocketPool,
        runtime: &libudx::UdxRuntime,
        remote_addr: &Ipv4Peer,
    ) {
        let target: SocketAddr = match format!("{}:{}", remote_addr.host, remote_addr.port).parse()
        {
            Ok(a) => a,
            Err(_) => return,
        };

        while self.punching && self.sockets.len() < BIRTHDAY_SOCKETS {
            match pool.acquire(runtime).await {
                Ok(mut socket) => {
                    let _ = socket.send_holepunch(target, true);
                    // Cover this birthday socket's probe stream with its own
                    // recv-adapter task so a probe landing on it (rather than
                    // the primary socket) still fires `Connected`.
                    if let Some(handle) = spawn_recv_adapter_for_socket(
                        &mut socket,
                        self.is_initiator,
                        self.event_tx.clone(),
                        Arc::clone(&self.connected_flag),
                    ) {
                        self.recv_tasks.push(handle);
                    }
                    self.sockets.push(socket);
                }
                Err(_) => break,
            }
        }
    }

    async fn keep_alive_random_nat(&mut self, remote_addr: &Ipv4Peer) {
        let target: SocketAddr = match format!("{}:{}", remote_addr.host, remote_addr.port).parse()
        {
            Ok(a) => a,
            Err(_) => return,
        };

        sleep(Duration::from_millis(100)).await;

        let mut i = 0;
        let mut low_ttl_rounds: u32 = 1;

        for _ in 0..1750 {
            if !self.punching {
                break;
            }

            if i == self.sockets.len() {
                i = 0;
                low_ttl_rounds = low_ttl_rounds.saturating_sub(1);
            }

            if let Some(socket) = self.sockets.get(i) {
                let _ = socket.send_holepunch(target, low_ttl_rounds > 0);
            }
            i += 1;

            if self.punching {
                sleep(Duration::from_millis(20)).await;
            }
        }

        self.auto_destroy();
    }

    /// **Note:** This method is retained for API compatibility but is not
    /// called by the production recv path. The internal recv-adapter task
    /// (spawned in `Holepuncher::new`) implements equivalent semantics with
    /// interior mutability via `connected_flag: Arc<AtomicBool>`.
    pub fn on_holepunch_message(&mut self, addr: SocketAddr, socket_idx: usize) {
        if !self.is_initiator {
            if let Some(socket) = self.sockets.get(socket_idx) {
                let _ = socket.send_holepunch(addr, false);
            }
            return;
        }

        if self.connected {
            return;
        }

        self.connected = true;
        self.punching = false;

        let _ = self.event_tx.send(HolepunchEvent::Connected { addr });
    }

    fn done(&self) -> bool {
        self.destroyed || self.connected
    }

    fn auto_destroy(&mut self) {
        if !self.connected {
            self.destroy();
        }
    }
}

/// Take ownership of `socket_ref`'s holepunch-probe receiver and spawn a
/// [`run_recv_adapter`] task for it, sharing the same `connected_flag` and
/// `event_tx` as every other socket belonging to the same [`Holepuncher`].
/// Every socket that can receive a `PEER_HOLEPUNCH` probe — the primary
/// socket acquired in `Holepuncher::new`, and each birthday-attack socket
/// opened by `open_birthday_sockets` — must be covered this way, or a probe
/// landing on an uncovered socket will silently never fire `Connected`.
///
/// Returns `None` (spawning nothing) if `socket_ref`'s `holepunch_rx` has
/// already been taken; this should not happen for a freshly acquired
/// `SocketRef` but is handled defensively rather than panicking.
fn spawn_recv_adapter_for_socket(
    socket_ref: &mut SocketRef,
    is_initiator: bool,
    event_tx: mpsc::UnboundedSender<HolepunchEvent>,
    connected_flag: Arc<AtomicBool>,
) -> Option<JoinHandle<()>> {
    let hp_rx = socket_ref.take_holepunch_rx()?;
    let socket_clone = socket_ref.socket.clone();
    Some(tokio::spawn(run_recv_adapter(
        hp_rx,
        socket_clone,
        is_initiator,
        event_tx,
        connected_flag,
    )))
}

async fn run_recv_adapter(
    mut hp_rx: mpsc::UnboundedReceiver<ProbeEvent>,
    socket: UdxSocket,
    is_initiator: bool,
    event_tx: mpsc::UnboundedSender<HolepunchEvent>,
    connected_flag: Arc<AtomicBool>,
) {
    while let Some(probe) = hp_rx.recv().await {
        if is_initiator {
            if connected_flag
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                tracing::info!(
                    target: "peeroxide::_events::holepunch::probe_received",
                    addr = %probe.addr,
                    "holepuncher(initiator): probe received, firing Connected"
                );
                let _ = event_tx.send(HolepunchEvent::Connected { addr: probe.addr });
                return;
            }
        } else {
            tracing::info!(
                target: "peeroxide::_events::holepunch::passive_reflected",
                addr = %probe.addr,
                "holepuncher(passive): probe received, reflecting"
            );
            let _ = socket.send_to(&[0u8], probe.addr);
        }
    }
}

pub fn match_address(
    local_addresses: &[Ipv4Peer],
    remote_local_addresses: &[Ipv4Peer],
) -> Option<Ipv4Peer> {
    if remote_local_addresses.is_empty() {
        return None;
    }

    let mut best_segment = 1u8;
    let mut best_addr: Option<&Ipv4Peer> = None;

    for local in local_addresses {
        let a: Vec<&str> = local.host.split('.').collect();
        if a.len() != 4 {
            continue;
        }

        for remote in remote_local_addresses {
            let b: Vec<&str> = remote.host.split('.').collect();
            if b.len() != 4 {
                continue;
            }

            if a[0] == b[0] {
                if best_segment == 1 {
                    best_segment = 2;
                    best_addr = Some(remote);
                }

                if a[1] == b[1] {
                    if best_segment == 2 {
                        best_segment = 3;
                        best_addr = Some(remote);
                    }

                    if a[2] == b[2] {
                        return Some(remote.clone());
                    }
                }
            }
        }
    }

    best_addr.cloned()
}

#[cfg(test)]
#[path = "holepuncher/match_address_tests.rs"]
mod match_address_tests;

#[cfg(test)]
#[path = "holepuncher/recv_adapter_tests.rs"]
mod recv_adapter_tests;

#[cfg(all(test, unix))]
#[path = "holepuncher/fd_tests.rs"]
mod fd_tests;
