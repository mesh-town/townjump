//! Route table: key -> records of socket + UDX peer address of a server.
// JS: hyperdht/lib/socket-pool.js
// Divergence from JS: socket-pool.js keeps one route per key and drops it on the first failure;
// here a key has several records and a failure counts. The wire does not change.

use std::sync::atomic::Ordering;

use libudx::UdxSocket;

use super::held::{Lease, find, take};
use super::{Reuse, lock};
use crate::messages::Ipv4Peer;

pub(super) struct Kept {
    pub(super) socket: UdxSocket,
    addr: Ipv4Peer,
    failures: u32,
}

/// A record goes after this many failures in a row, whatever its socket does.
const MAX_FAILURES: u32 = 2;

/// A remembered route to a server; `lease` is set when its socket is held.
pub(crate) struct Route {
    pub socket: UdxSocket,
    pub addr: Ipv4Peer,
    pub lease: Option<Lease>,
}

impl Reuse {
    /// Remember a route: a new socket adds a record, a known one resets its failures. For a
    /// punch socket call it only while holding a lease, or the socket may expire and leave
    /// the record without a reader.
    pub(crate) fn add_route(&self, key: [u8; 32], socket: &UdxSocket, addr: Ipv4Peer) {
        let mut st = lock(&self.inner);
        if st.closed {
            return;
        }
        let records = st.routes.entry(key).or_default();
        match records.iter_mut().find(|k| k.socket.ptr_eq(socket)) {
            Some(known) => known.failures = 0,
            None => records.push(Kept {
                socket: socket.clone(),
                addr,
                failures: 0,
            }),
        }
    }

    /// The record with the fewest failures; the last added wins a tie.
    pub(crate) fn route(&self, key: &[u8; 32]) -> Option<Route> {
        let mut st = lock(&self.inner);
        let (socket, addr) = {
            let k = st
                .routes
                .get(key)?
                .iter()
                .rev()
                .min_by_key(|k| k.failures)?;
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

    /// A retry over the record of `key` on `socket` failed; `own_lease`: the attempt itself
    /// holds a lease on `socket`. The record is dropped on its second failure in a row or when
    /// no other lease keeps the socket alive (a socket not held, like the node's, has none).
    pub(crate) fn route_failed(&self, key: &[u8; 32], socket: &UdxSocket, own_lease: bool) {
        let mut st = lock(&self.inner);
        let others =
            find(&st, socket).map(|i| st.held[i].leases.saturating_sub(usize::from(own_lease)));
        let Some(records) = st.routes.get_mut(key) else {
            return;
        };
        let Some(at) = records.iter().position(|k| k.socket.ptr_eq(socket)) else {
            return;
        };
        records[at].failures += 1;
        let failures = records[at].failures;
        if failures < MAX_FAILURES && others != Some(0) {
            tracing::debug!(key = ?&key[..4], failures, "reuse: route kept after failure");
            return;
        }
        let gone = records.remove(at);
        if records.is_empty() {
            st.routes.remove(key);
        }
        drop(st);
        self.inner.drops.fetch_add(1, Ordering::Relaxed);
        tracing::debug!(key = ?&key[..4], "reuse: route dropped");
        drop(gone);
    }

    /// A retry over the record of `key` on `socket` produced a connection.
    pub(crate) fn route_succeeded(&self, key: &[u8; 32], socket: &UdxSocket) {
        let mut st = lock(&self.inner);
        let found = st.routes.get_mut(key);
        if let Some(k) = found.and_then(|r| r.iter_mut().find(|k| k.socket.ptr_eq(socket))) {
            k.failures = 0;
        }
    }
}
