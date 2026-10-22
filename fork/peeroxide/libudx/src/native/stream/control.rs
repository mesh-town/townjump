// JS: libudx/src/udx.c (udx_stream_change_remote)
//! `StreamControl`: management of a stream after `into_async_stream` — route change, remote address, closing.

use super::upgrade::table_entry;
use super::*;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

/// A cloneable control handle of a stream; take it with `UdxStream::control` before `into_async_stream`.
#[derive(Clone)]
pub struct StreamControl {
    inner: Arc<Mutex<StreamInner>>,
    tx: mpsc::WeakUnboundedSender<IncomingPacket>,
    local_id: u32,
}

/// A read-only, cloneable view of a stream's remote address; take it with `UdxAsyncStream::peer` before the stream is
/// split or moved into a task.
#[derive(Clone)]
pub struct StreamPeer(pub(crate) Arc<Mutex<StreamInner>>);

impl StreamPeer {
    /// The peer's address and stream id; `Some` while the address is known, also after closing.
    pub fn remote(&self) -> Option<(SocketAddr, u32)> {
        let inner = self.0.lock().unwrap_or_else(|e| e.into_inner());
        inner.remote_addr.map(|addr| (addr, inner.remote_id))
    }

    /// The local address of the socket the stream currently sends from (follows `change_remote`).
    pub fn local(&self) -> Option<SocketAddr> {
        let inner = self.0.lock().unwrap_or_else(|e| e.into_inner());
        inner.udp.as_ref()?.local_addr().ok()
    }
}

/// Completion of `change_remote`: ready at once, or when the peer ACKs everything sent before the change.
#[must_use]
pub struct RemoteChanged(Option<oneshot::Receiver<()>>);

impl Future for RemoteChanged {
    type Output = Result<()>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let Some(rx) = self.0.as_mut() else {
            return Poll::Ready(Ok(()));
        };
        match Pin::new(rx).poll(cx) {
            Poll::Ready(Ok(())) => Poll::Ready(Ok(())),
            Poll::Ready(Err(_)) => Poll::Ready(Err(UdxError::StreamClosed)),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl UdxStream {
    /// A control handle; the stream's `Arc` state is shared, so it outlives `into_async_stream`.
    pub fn control(&self) -> StreamControl {
        let tx = self.incoming_tx.as_ref().map(|t| t.downgrade());
        StreamControl {
            inner: Arc::clone(&self.inner),
            tx: tx.unwrap_or_else(|| mpsc::unbounded_channel().0.downgrade()),
            local_id: self.local_id,
        }
    }
}

impl StreamControl {
    fn lock(&self) -> std::sync::MutexGuard<'_, StreamInner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Moves the stream to `remote_addr` / `remote_id` through `socket`. Packets already sent keep their old route; the
    /// stream stays in the old socket's table too (the old path still delivers). The result is ready at once if nothing is
    /// in flight, else when the peer ACKs everything sent before the change; it fails with `StreamClosed` on close.
    /// Errors: closed, not connected yet, a change already pending, port 0, the stream id taken on `socket`.
    /// `socket` must belong to the same `UdxRuntime` as the stream: the caller's responsibility; it cannot be checked here.
    pub fn change_remote(
        &self,
        socket: &super::socket::UdxSocket,
        remote_id: u32,
        remote_addr: SocketAddr,
    ) -> Result<RemoteChanged> {
        if remote_addr.port() == 0 {
            let invalid = std::io::Error::new(std::io::ErrorKind::InvalidInput, "remote port is 0");
            return Err(invalid.into());
        }
        let tx = self.tx.upgrade().ok_or(UdxError::StreamClosed)?;
        self.lock().check_changeable()?;
        let udp = socket.udp_arc()?;
        let entry = table_entry(socket, self.local_id, &tx)?;
        let mut inner = self.lock();
        inner.check_changeable()?;
        let waiting = inner.begin_change(udp, remote_id, remote_addr);
        inner.extra_tables.extend(entry);
        Ok(RemoteChanged(waiting))
    }

    /// Sends a MESSAGE packet (type 0x08) to the current remote, in the background; it is not queued or retransmitted
    /// and does not advance the sequence number. `false` - not connected or closed.
    pub fn try_send_message(&self, payload: &[u8]) -> Result<bool> {
        Ok(self.lock().send_message(payload))
    }

    /// Resolves after the next DATA or MESSAGE packet from the current remote address (packets seen before the call do
    /// not count); fails at once on a closed stream and when the stream closes first.
    pub fn await_direct(&self) -> oneshot::Receiver<()> {
        self.lock().add_direct_waiter()
    }

    /// Vets packets arriving from a source other than `remote_addr` on a connected stream: `true` - process, `false` -
    /// drop. A repeated call replaces the hook. The hook runs on the processor task without locks held, must not
    /// block, and may call this handle's methods (including `change_remote`).
    pub fn set_path_hook(&self, f: impl Fn(SocketAddr) -> bool + Send + Sync + 'static) {
        self.lock().path_hook = Some(Arc::new(f));
    }

    /// The peer's address and stream id; `Some` while the address is known, also after closing.
    pub fn remote(&self) -> Option<(SocketAddr, u32)> {
        let inner = self.lock();
        inner.remote_addr.map(|addr| (addr, inner.remote_id))
    }

    /// Resolves when the stream is closed (processor ended, `destroy`, or the handle dropped before start).
    pub fn closed(&self) -> impl Future<Output = ()> + Send + 'static {
        let mut rx = self.lock().closed.subscribe();
        async move {
            let _ = rx.wait_for(|closed| *closed).await;
        }
    }
}
