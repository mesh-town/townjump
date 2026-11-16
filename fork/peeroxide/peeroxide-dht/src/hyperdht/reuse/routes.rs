//! Route table: key -> socket + UDX peer address of a server.
// JS: hyperdht/lib/socket-pool.js

use std::sync::atomic::Ordering;

use libudx::UdxSocket;

use super::held::{Lease, find, take};
use super::{Reuse, lock};
use crate::messages::Ipv4Peer;

pub(super) struct Kept {
    pub(super) socket: UdxSocket,
    addr: Ipv4Peer,
}

/// A remembered route to a server; `lease` is set when its socket is held.
pub(crate) struct Route {
    pub socket: UdxSocket,
    pub addr: Ipv4Peer,
    pub lease: Option<Lease>,
}

impl Reuse {
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

    /// A retry over the route of `key` on `socket` failed; `own_lease`: the attempt holds a lease.
    pub(crate) fn route_failed(&self, key: &[u8; 32], socket: &UdxSocket, _own_lease: bool) {
        self.drop_route(key, socket);
    }

    /// A retry over the route of `key` on `socket` produced a connection.
    pub(crate) fn route_succeeded(&self, _key: &[u8; 32], _socket: &UdxSocket) {}
}
