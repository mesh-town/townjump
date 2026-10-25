// JS: encrypted stream `c` of a hyperdht connection. Here the fork `PeerConnection` → `Box<dyn Duplex>` without pump tasks.
// See also: slot.rs (connection owner), tunnels.rs (registry); dht.rs and server.rs hand out streams via `Node::adopt`.
use crate::slot::{Side, Slot};
use peeroxide_dht::hyperdht::PeerConnection;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use townjump_dht::{DhtConnection, RttProbe};

/// The whole fork connection in a slot: the UDX socket is kept alive as long as the stream; reads and writes go straight to `SecretStream`.
struct PeerStream {
    slot: Arc<Slot>,
}

impl AsyncRead for PeerStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let read =
            |stream: Pin<&mut _>, cx: &mut Context<'_>| AsyncRead::poll_read(stream, cx, buf);
        self.slot.poll_with(Side::Read, cx, read)
    }
}

impl AsyncWrite for PeerStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let write =
            |stream: Pin<&mut _>, cx: &mut Context<'_>| AsyncWrite::poll_write(stream, cx, buf);
        self.slot.poll_with(Side::Write, cx, write)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.slot.poll_with(Side::Write, cx, AsyncWrite::poll_flush)
    }

    /// The abort has already done its job: `end()` on a destroyed stream is silent in JS.
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.slot.is_killed() {
            return Poll::Ready(Ok(()));
        }
        self.slot
            .poll_with(Side::Write, cx, AsyncWrite::poll_shutdown)
    }
}

/// Wraps the whole fork connection: peer key and stream (`shutdown` = END, drop before end of stream = DESTROY).
/// Outside the tunnel registry: no abort on `suspend`; the node hands out streams via `Tunnels::register`.
///
/// `DhtConnection::new`, not `silent_close`: a real stream sends `close` on `destroy()`.
pub fn wrap_connection(conn: PeerConnection) -> DhtConnection {
    wrap_slot(Arc::new(Slot::new(conn)))
}

pub(crate) fn wrap_slot(slot: Arc<Slot>) -> DhtConnection {
    let remote_public_key = slot.remote_public_key;
    let remote_addr = slot.remote_addr;
    let weak = Arc::downgrade(&slot);
    let probe: RttProbe = Arc::new(move || weak.upgrade()?.rtt());
    DhtConnection::new(remote_public_key, Box::new(PeerStream { slot }))
        .with_remote_addr(remote_addr)
        .with_rtt_probe(probe)
}

#[cfg(test)]
#[path = "stream_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "stream_meta_tests.rs"]
mod meta_tests;
