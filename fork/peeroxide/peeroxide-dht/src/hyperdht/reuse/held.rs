//! Held punch sockets: leases, the linger timer and the datagram forwarder.
// JS: hyperdht/lib/socket-pool.js

use std::sync::{Arc, Weak};

use libudx::{Datagram, UdxSocket};
use tokio::runtime::Handle;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::task::JoinHandle;

use super::routes::Kept;
use super::{Inner, LINGER, Reuse, State, lock};

pub(super) struct Held {
    pub(super) slot: u64,
    pub(super) socket: UdxSocket,
    pub(super) leases: usize,
    pub(super) generation: u64,
    forwarder: JoinHandle<()>,
}

impl Drop for Held {
    fn drop(&mut self) {
        self.forwarder.abort();
    }
}

/// One claim on a held socket; the socket lingers `LINGER` after the last one is dropped.
pub(crate) struct Lease {
    inner: Weak<Inner>,
    slot: u64,
}

impl Drop for Lease {
    fn drop(&mut self) {
        if let Some(inner) = self.inner.upgrade() {
            release(&inner, self.slot);
        }
    }
}

pub(super) fn find(state: &State, socket: &UdxSocket) -> Option<usize> {
    state.held.iter().position(|h| h.socket.ptr_eq(socket))
}

pub(super) fn take(inner: &Arc<Inner>, held: &mut Held) -> Lease {
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

    /// Observe without leasing (does not re-arm the timer).
    #[cfg(test)]
    pub(crate) fn is_held(&self, socket: &UdxSocket) -> bool {
        find(&lock(&self.inner), socket).is_some()
    }
}
