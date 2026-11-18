//! Held punch sockets and server routes (key -> socket + UDX peer address), with a linger timer.
//! Neighbours: `connection` carries the `Lease`; `handle` owns the `Reuse`.
// JS: hyperdht/lib/socket-pool.js

use portable_atomic::AtomicU64;
use std::collections::HashMap;
use std::net::SocketAddr;
#[cfg(test)]
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use libudx::UdxSocket;

mod held;
mod routes;

use held::Held;
pub(crate) use held::Lease;
use routes::Kept;

/// Delivers a datagram received on a held socket to the node (`SocketAddr` = sender).
pub(crate) type Forward = Arc<dyn Fn(SocketAddr, Vec<u8>, UdxSocket) + Send + Sync>;

/// How long a socket stays open after its last lease is released.
pub(crate) const LINGER: Duration = Duration::from_millis(3000); // JS: socket-pool.js:3

struct State {
    held: Vec<Held>,
    routes: HashMap<[u8; 32], Vec<Kept>>,
    next_slot: u64,
    closed: bool,
}

struct Inner {
    forward: Forward,
    state: Mutex<State>,
    hits: AtomicU64,
    drops: AtomicU64,
    /// Test hook: the next route retry fails before its handshake.
    #[cfg(test)]
    fail_next_route: AtomicBool,
}

/// Socket-pool store; clones share one state. Timers, forwarders and leases hold only `Weak`.
#[derive(Clone)]
pub(crate) struct Reuse {
    inner: Arc<Inner>,
}

fn lock(inner: &Inner) -> MutexGuard<'_, State> {
    inner.state.lock().unwrap_or_else(PoisonError::into_inner)
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
                #[cfg(test)]
                fail_next_route: AtomicBool::new(false),
            }),
        }
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

    /// Test hook: make the next route retry fail before its handshake.
    #[cfg(test)]
    pub(crate) fn fail_next_route(&self) {
        self.inner.fail_next_route.store(true, Ordering::Relaxed);
    }

    /// Whether the injected failure is armed; disarms it.
    #[cfg(test)]
    pub(crate) fn take_fail_next_route(&self) -> bool {
        self.inner.fail_next_route.swap(false, Ordering::Relaxed)
    }

    /// Test view: the sockets of the records kept for `key`.
    #[cfg(test)]
    pub(crate) fn route_sockets(&self, key: &[u8; 32]) -> Vec<UdxSocket> {
        let st = lock(&self.inner);
        let records = st.routes.get(key);
        records.map_or_else(Vec::new, |r| r.iter().map(|k| k.socket.clone()).collect())
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn hits(&self) -> u64 {
        self.inner.hits.load(Ordering::Relaxed)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn drops(&self) -> u64 {
        self.inner.drops.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
