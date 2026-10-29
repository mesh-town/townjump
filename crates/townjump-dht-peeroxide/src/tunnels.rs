// JS: `dht.rawStreams` (hyperdht/index.js) — the node's stream registry that `suspend()` clears.
// See also: slot.rs (what lies in the registry), node_suspend.rs (who calls `close_all`), stream.rs (`wrap_slot`).
use crate::path::bridge_path;
use crate::slot::Slot;
use crate::stream::wrap_slot;
use peeroxide_dht::hyperdht::PeerConnection;
use std::sync::{Arc, Mutex, Weak};
use townjump_dht::DhtConnection;

/// Tunnels handed upward: weak references, the consumer owns the stream (dropping the stream frees the slot).
#[derive(Default)]
pub(crate) struct Tunnels {
    slots: Mutex<Vec<Weak<Slot>>>,
}

impl Tunnels {
    /// Fork connection → stream for the consumer (keepalive on), the slot is recorded in the registry; dead entries are purged.
    pub fn register(&self, conn: PeerConnection) -> DhtConnection {
        let path = conn.relay_path();
        let slot = Arc::new(Slot::new(conn));
        let mut slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        slots.retain(|w| w.strong_count() > 0);
        slots.push(Arc::downgrade(&slot));
        drop(slots);
        let wrapped = wrap_slot(slot);
        match path {
            Some(rx) => wrapped.with_relay_path(bridge_path(rx)),
            None => wrapped,
        }
    }

    /// JS `rawStreams.clear()`: tears down all live tunnels, the registry empties.
    pub fn close_all(&self) {
        let mut slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        let alive: Vec<Arc<Slot>> = slots.drain(..).filter_map(|w| w.upgrade()).collect();
        drop(slots);
        alive.iter().for_each(|slot| slot.kill());
    }

    /// How many tunnels are still in the registry and not aborted (for tests).
    #[cfg(test)]
    pub fn open(&self) -> usize {
        let slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        let alive = slots.iter().filter_map(Weak::upgrade);
        alive.filter(|slot| !slot.is_killed()).count()
    }
}

#[cfg(test)]
#[path = "tunnels_tests.rs"]
mod tests;
