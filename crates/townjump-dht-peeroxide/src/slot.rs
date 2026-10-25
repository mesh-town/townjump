// JS: connection `c` from `dht.rawStreams` (hyperdht/index.js) — what `suspend()` tears down via `rawStreams.clear()`.
// See also: tunnels.rs (slot registry), stream.rs (`PeerStream` reads and writes through the slot).
use libudx::UdxAsyncStream;
use peeroxide_dht::hyperdht::PeerConnection;
use peeroxide_dht::secret_stream::SecretStream;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Mutex, MutexGuard};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

/// hyperdht `connectionKeepAlive`: an empty message after 5 s without a write (client and server).
pub(crate) const KEEP_ALIVE: Duration = Duration::from_secs(5);

type Secret = SecretStream<UdxAsyncStream>;

/// Who waits on the slot: the reader and the writer are different tasks (`tokio::io::split`), each with its own Waker.
#[derive(Clone, Copy)]
pub(crate) enum Side {
    Read,
    Write,
}

#[derive(Default)]
struct Wakers {
    read: Option<Waker>,
    write: Option<Waker>,
}

/// A fork connection the adapter may abort on its own: `kill` drops it (DESTROY to the remote peer) and wakes the waiters.
pub(crate) struct Slot {
    conn: Mutex<Option<PeerConnection>>,
    wakers: Mutex<Wakers>,
    pub remote_public_key: [u8; 32],
    pub remote_addr: Option<SocketAddr>,
}

impl Slot {
    /// Enables the 5 s keepalive explicitly, not relying on the fork default. Needs a tokio runtime.
    pub fn new(mut conn: PeerConnection) -> Slot {
        conn.stream.set_keep_alive(KEEP_ALIVE);
        Slot {
            remote_public_key: conn.remote_public_key,
            remote_addr: conn.remote_addr,
            conn: Mutex::new(Some(conn)),
            wakers: Mutex::new(Wakers::default()),
        }
    }

    pub fn is_killed(&self) -> bool {
        lock(&self.conn).is_none()
    }

    /// Smoothed round-trip time of the live connection; `None` before the first sample and after `kill`.
    pub fn rtt(&self) -> Option<Duration> {
        let guard = lock(&self.conn);
        let srtt_ms = guard.as_ref()?.stream.get_ref().stats().srtt_ms;
        (srtt_ms != 0).then(|| Duration::from_millis(u64::from(srtt_ms)))
    }

    /// Polls the stream under the slot lock; after an abort — `ConnectionAborted`. The lock is held until the Waker is recorded,
    /// so that a `kill` landing between the poll and the record does not miss the waiter.
    pub fn poll_with<T>(
        &self,
        side: Side,
        cx: &mut Context<'_>,
        poll: impl FnOnce(Pin<&mut Secret>, &mut Context<'_>) -> Poll<io::Result<T>>,
    ) -> Poll<io::Result<T>> {
        let mut guard = lock(&self.conn);
        let Some(conn) = guard.as_mut() else {
            return Poll::Ready(Err(aborted()));
        };
        let polled = poll(Pin::new(&mut conn.stream), cx);
        if polled.is_pending() {
            self.remember(side, cx);
        }
        polled
    }

    /// Drops the connection (dropping `UdxAsyncStream` before end of stream sends DESTROY) and wakes the reader and the writer.
    pub fn kill(&self) {
        let conn = lock(&self.conn).take();
        drop(conn);
        let wakers = std::mem::take(&mut *lock(&self.wakers));
        [wakers.read, wakers.write]
            .into_iter()
            .flatten()
            .for_each(Waker::wake);
    }

    fn remember(&self, side: Side, cx: &Context<'_>) {
        let mut wakers = lock(&self.wakers);
        let waker = Some(cx.waker().clone());
        match side {
            Side::Read => wakers.read = waker,
            Side::Write => wakers.write = waker,
        }
    }
}

/// Local analogue of JS `destroy()`: the consumer's stream is aborted, not finished (error ≠ EOF).
pub(crate) fn aborted() -> io::Error {
    io::Error::new(io::ErrorKind::ConnectionAborted, "tunnel closed by suspend")
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
#[path = "slot_tests.rs"]
mod tests;
