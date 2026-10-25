// JS: dht-rpc/index.js Session (`dht.session()`), dht-rpc/lib/io.js Request.destroy
//! `struct Session`: a scope for DHT requests and queries; dropping or destroying it cancels
//! everything still running in it. Neighbours: `handle` creates sessions and sends the
//! `DestroySession` command that `node_query::destroy_session` executes.

use portable_atomic::AtomicU64;
use std::sync::atomic::Ordering;

use super::command::DhtCommand;
use super::handle::DhtHandle;

/// Ids of sessions, process-wide; 0 means "no session".
static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

/// Scope of DHT requests: calls made through [`Session::dht`] belong to it, and
/// [`Session::destroy`] (also on drop) stops them (no retries, no timeouts, callers get an
/// error). The routing table is not touched. The session stays usable after `destroy`.
pub struct Session {
    dht: DhtHandle,
}

impl Session {
    pub(super) fn new(handle: &DhtHandle) -> Self {
        let id = NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed);
        Self {
            dht: DhtHandle {
                session_id: Some(id),
                ..handle.clone()
            },
        }
    }

    /// Handle whose `find_node`, `query` and `request` run in this session.
    pub fn dht(&self) -> &DhtHandle {
        &self.dht
    }

    /// Cancels everything running in the session so far; a closed node is ignored.
    pub fn destroy(&self) {
        if let Some(id) = self.dht.session_id {
            let _ = self.dht.cmd_tx.send(DhtCommand::DestroySession { id });
        }
    }

    #[cfg(test)]
    pub(crate) fn id(&self) -> u64 {
        self.dht.session_id.unwrap_or(0)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.destroy();
    }
}
