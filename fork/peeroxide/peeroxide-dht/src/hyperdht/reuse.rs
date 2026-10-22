//! Held punch sockets and server routes (key -> socket + UDX peer address), with a linger timer.
//! Neighbours: `connection` carries the `Lease`; `handle` owns the `Reuse`.
// JS: hyperdht/lib/socket-pool.js

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use libudx::{Datagram, UdxSocket};
use tokio::runtime::Handle;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::task::JoinHandle;

use crate::messages::Ipv4Peer;

/// Delivers a datagram received on a held socket to the node (`SocketAddr` = sender).
pub(crate) type Forward = Arc<dyn Fn(SocketAddr, Vec<u8>, UdxSocket) + Send + Sync>;

/// How long a socket stays open after its last lease is released.
pub(crate) const LINGER: Duration = Duration::from_millis(3000); // JS: socket-pool.js:3

struct Held {
    slot: u64,
    socket: UdxSocket,
    leases: usize,
    generation: u64,
    forwarder: JoinHandle<()>,
}

impl Drop for Held {
    fn drop(&mut self) {
        self.forwarder.abort();
    }
}

struct Kept {
    socket: UdxSocket,
    addr: Ipv4Peer,
}

struct State {
    held: Vec<Held>,
    routes: HashMap<[u8; 32], Kept>,
    next_slot: u64,
    closed: bool,
}

struct Inner {
    forward: Forward,
    state: Mutex<State>,
    hits: AtomicU64,
    drops: AtomicU64,
}

/// Socket-pool store; clones share one state. Timers, forwarders and leases hold only `Weak`.
#[derive(Clone)]
pub(crate) struct Reuse {
    inner: Arc<Inner>,
}

/// One claim on a held socket; the socket lingers `LINGER` after the last one is dropped.
pub(crate) struct Lease {
    inner: Weak<Inner>,
    slot: u64,
}

/// A remembered route to a server; `lease` is set when its socket is held.
pub(crate) struct Route {
    pub socket: UdxSocket,
    pub addr: Ipv4Peer,
    pub lease: Option<Lease>,
}

impl Drop for Lease {
    fn drop(&mut self) {
        if let Some(inner) = self.inner.upgrade() {
            release(&inner, self.slot);
        }
    }
}

fn lock(inner: &Inner) -> MutexGuard<'_, State> {
    inner.state.lock().unwrap_or_else(PoisonError::into_inner)
}

fn find(state: &State, socket: &UdxSocket) -> Option<usize> {
    state.held.iter().position(|h| h.socket.ptr_eq(socket))
}

fn take(inner: &Arc<Inner>, held: &mut Held) -> Lease {
    held.leases += 1;
    Lease {
        inner: Arc::downgrade(inner),
        slot: held.slot,
    }
}

/// Removes the entry and its routes; the caller drops them after the lock (socket close).
fn expire(state: &mut State, slot: u64) -> Option<(Held, Vec<Kept>)> {
    let held = state
        .held
        .remove(state.held.iter().position(|h| h.slot == slot)?);
    let keys: Vec<[u8; 32]> = (state.routes.iter())
        .filter(|(_, k)| k.socket.ptr_eq(&held.socket))
        .map(|(key, _)| *key)
        .collect();
    let gone = keys.iter().filter_map(|k| state.routes.remove(k)).collect();
    Some((held, gone))
}

fn spawn_forwarder(
    h: &Handle,
    inner: Weak<Inner>,
    slot: u64,
    mut rx: UnboundedReceiver<Datagram>,
) -> JoinHandle<()> {
    h.spawn(async move {
        while let Some(dg) = rx.recv().await {
            // A one-byte datagram is a holepunch probe (JS socket-pool.js:144-150).
            if dg.data.len() <= 1 {
                continue;
            }
            let Some(inner) = inner.upgrade() else { return };
            let socket = {
                let st = lock(&inner);
                st.held
                    .iter()
                    .find(|h| h.slot == slot)
                    .map(|h| h.socket.clone())
            };
            let Some(socket) = socket else { return };
            (inner.forward)(dg.addr, dg.data, socket);
        }
    })
}

fn release(inner: &Arc<Inner>, slot: u64) {
    let generation = {
        let mut st = lock(inner);
        let Some(held) = st.held.iter_mut().find(|h| h.slot == slot) else {
            return;
        };
        held.leases = held.leases.saturating_sub(1);
        if held.leases > 0 {
            return;
        }
        held.generation += 1;
        held.generation
    };
    if let Ok(h) = Handle::try_current() {
        h.spawn(linger(Arc::downgrade(inner), slot, generation));
        return;
    }
    let gone = expire(&mut lock(inner), slot);
    log_released(gone);
}

fn log_released(gone: Option<(Held, Vec<Kept>)>) {
    if let Some((h, routes)) = gone {
        tracing::debug!(
            slot = h.slot,
            routes = routes.len(),
            "reuse: socket released"
        );
    }
}

async fn linger(inner: Weak<Inner>, slot: u64, generation: u64) {
    tokio::time::sleep(LINGER).await;
    let Some(inner) = inner.upgrade() else { return };
    let gone = {
        let mut st = lock(&inner);
        let due = st
            .held
            .iter()
            .any(|h| h.slot == slot && h.leases == 0 && h.generation == generation);
        if due { expire(&mut st, slot) } else { None }
    };
    log_released(gone);
}

impl Reuse {
    pub(crate) fn new(forward: Forward) -> Reuse {
        Reuse {
            inner: Arc::new(Inner {
                forward,
                state: Mutex::new(State {
                    held: Vec::new(),
                    routes: HashMap::new(),
                    next_slot: 0,
                    closed: false,
                }),
                hits: AtomicU64::new(0),
                drops: AtomicU64::new(0),
            }),
        }
    }

    /// Hold a punch socket after the connect (never the node's IO socket: it would steal
    /// the node's receive path). `None`: cleared, no runtime, or `recv_start` failed.
    pub(crate) fn hold(&self, socket: &UdxSocket) -> Option<Lease> {
        let mut st = lock(&self.inner);
        if st.closed {
            return None;
        }
        if let Some(i) = find(&st, socket) {
            return Some(take(&self.inner, &mut st.held[i]));
        }
        let h = Handle::try_current().ok()?;
        let rx = (socket.recv_start())
            .inspect_err(|e| tracing::debug!("reuse: recv_start failed: {e}"))
            .ok()?;
        let slot = st.next_slot;
        st.next_slot += 1;
        let forwarder = spawn_forwarder(&h, Arc::downgrade(&self.inner), slot, rx);
        let mut held = Held {
            slot,
            socket: socket.clone(),
            leases: 0,
            generation: 0,
            forwarder,
        };
        let lease = take(&self.inner, &mut held);
        st.held.push(held);
        tracing::debug!(slot, "reuse: socket held");
        Some(lease)
    }

    /// A lease on an already held socket; `None` for any other (e.g. the IO socket).
    pub(crate) fn lease(&self, socket: &UdxSocket) -> Option<Lease> {
        let mut st = lock(&self.inner);
        if st.closed {
            return None;
        }
        let i = find(&st, socket)?;
        Some(take(&self.inner, &mut st.held[i]))
    }

    /// Remember a route; an existing one is kept. For a punch socket call it only while
    /// holding a lease, or the socket may expire and leave the route without a reader.
    pub(crate) fn add_route(&self, key: [u8; 32], socket: &UdxSocket, addr: Ipv4Peer) {
        let mut st = lock(&self.inner);
        if st.closed {
            return;
        }
        st.routes.entry(key).or_insert_with(|| Kept {
            socket: socket.clone(),
            addr,
        });
    }

    pub(crate) fn route(&self, key: &[u8; 32]) -> Option<Route> {
        let mut st = lock(&self.inner);
        let (socket, addr) = {
            let k = st.routes.get(key)?;
            (k.socket.clone(), k.addr.clone())
        };
        self.inner.hits.fetch_add(1, Ordering::Relaxed);
        tracing::debug!(key = ?&key[..4], "reuse: route hit");
        let lease = find(&st, &socket).map(|i| take(&self.inner, &mut st.held[i]));
        Some(Route {
            socket,
            addr,
            lease,
        })
    }

    /// Forget the route of `key` if it uses `socket`; the held entry is left alone.
    pub(crate) fn drop_route(&self, key: &[u8; 32], socket: &UdxSocket) {
        let mut st = lock(&self.inner);
        if !st.routes.get(key).is_some_and(|k| k.socket.ptr_eq(socket)) {
            return;
        }
        let gone = st.routes.remove(key);
        drop(st);
        self.inner.drops.fetch_add(1, Ordering::Relaxed);
        tracing::debug!(key = ?&key[..4], "reuse: route dropped");
        drop(gone);
    }

    /// Terminal: drops every route, held socket and forwarder; later calls find nothing.
    pub(crate) fn clear(&self) {
        let (held, routes) = {
            let mut st = lock(&self.inner);
            st.closed = true;
            (std::mem::take(&mut st.held), std::mem::take(&mut st.routes))
        };
        drop((held, routes));
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn hits(&self) -> u64 {
        self.inner.hits.load(Ordering::Relaxed)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn drops(&self) -> u64 {
        self.inner.drops.load(Ordering::Relaxed)
    }

    /// Observe without leasing (does not re-arm the timer).
    #[cfg(test)]
    pub(crate) fn is_held(&self, socket: &UdxSocket) -> bool {
        find(&lock(&self.inner), socket).is_some()
    }
}

#[cfg(test)]
#[path = "reuse_tests.rs"]
mod tests;
