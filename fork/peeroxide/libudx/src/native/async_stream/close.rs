//! Close: drop before the stream ends and `abort()` send DESTROY; a finished stream closes via FIN.
// JS: udx-native/lib/stream.js

use std::net::SocketAddr;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::runtime::Handle;
use tokio::task::JoinHandle;

use super::adapter::UdxAsyncStream;
use super::read::ReadState;
use crate::native::stream::{DRAIN_MAX, DRAIN_MIN, StreamInner, StreamNotify};

impl UdxAsyncStream {
    /// Aborts the stream: the peer gets DESTROY (`ECONNRESET`) if the stream is still `connected`
    /// — even after `shutdown`, like `destroy()` after `end()` in JS. Drop after `abort`
    /// sends nothing.
    pub fn abort(mut self) {
        self.send_destroy();
        if let Some(processor) = self.processor.take() {
            processor.abort();
        }
    }

    /// The stream is finished: our FIN is acked and the peer's FIN received, or the stream is already torn down.
    fn is_finished(&self) -> bool {
        if self.write_done && self.read_state == ReadState::Eof {
            return true;
        }
        !self
            .inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .connected
    }

    /// There is data in the send queue or awaiting an ACK: a direct DESTROY would overtake it (`write` only queues packets,
    /// the processor task sends them), and `abort()` would lose the processor.
    fn has_unsent(&self) -> bool {
        !self
            .inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .send_idle()
    }

    /// DESTROY goes into the send queue behind the data: the processor sends everything in order and exits
    /// (`notify_shutdown`), so the socket-map entry is removed normally.
    fn queue_destroy(&mut self) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, addr, bytes)) = inner.destroy_packet() {
            let seq = inner.next_seq;
            inner.queue_for_send(vec![(seq, bytes)], addr);
        }
    }

    fn notify_shutdown(&self) {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(ref tx) = guard.notify_tx {
            let _ = tx.send(StreamNotify::Shutdown);
        }
    }

    /// A single DESTROY packet without retransmission, like `UdxStream::destroy` and JS; `connected == false` — nothing.
    fn send_destroy(&mut self) {
        let packet = {
            let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            inner.destroy_packet()
        };
        if let Some((udp, addr, bytes)) = packet {
            send_best_effort(udp, bytes, addr);
        }
    }
}

/// How long a dropped adapter may keep draining its unsent data: `clamp(4 * rto, 2 s, 10 s)`.
pub(super) fn drain_limit(rto: Duration) -> Duration {
    rto.saturating_mul(4).clamp(DRAIN_MIN, DRAIN_MAX)
}

/// Waits for the processor to drain; at the limit it gives up on the unsent data, sends DESTROY and aborts the
/// processor (the same end as the direct path), so a peer with a closed window cannot hold the stream forever.
async fn drain_watch(
    mut processor: JoinHandle<()>,
    inner: Weak<Mutex<StreamInner>>,
    limit: Duration,
) {
    if tokio::time::timeout(limit, &mut processor).await.is_ok() {
        return;
    }
    let packet = inner.upgrade().and_then(|inner| {
        let mut inner = inner.lock().unwrap_or_else(|e| e.into_inner());
        let packet = inner.destroy_packet();
        inner.abandon_unsent();
        packet
    });
    if let Some((udp, addr, bytes)) = packet {
        send_best_effort(udp, bytes, addr);
    }
    processor.abort();
}

/// Sends without `async`: tries `try_send_to` first; on `WouldBlock` or an error, and when inside a runtime, falls back to `spawn(send_to)`.
fn send_best_effort(udp: Arc<UdpSocket>, bytes: Vec<u8>, addr: SocketAddr) {
    if udp.try_send_to(&bytes, addr).is_ok() {
        return;
    }
    if let Ok(handle) = Handle::try_current() {
        handle.spawn(async move {
            let _sent = crate::native::tx_err::send_counted(&udp, &bytes, addr, "close").await;
        });
    }
}

/// A graceful close is `shutdown()` and waiting for EOF before the drop; dropping with unsent data and then receiving
/// data from the peer resets the stream and discards the unsent tail, like a TCP RST.
///
/// Three paths: stream finished — quiet processor exit; unsent data — DESTROY behind the data, with a watcher that
/// ends the drain at `drain_limit`; otherwise (or without a runtime) — direct DESTROY and `abort()` of the processor.
impl Drop for UdxAsyncStream {
    fn drop(&mut self) {
        let Some(processor) = self.processor.take() else {
            self.notify_shutdown();
            return;
        };
        let rto = {
            let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            inner.read_closed = true;
            Duration::from_millis(u64::from(inner.rto))
        };
        if self.is_finished() {
            drop(processor);
            self.notify_shutdown();
            return;
        }
        if self.has_unsent()
            && let Ok(runtime) = Handle::try_current()
        {
            self.queue_destroy();
            self.notify_shutdown();
            let weak = Arc::downgrade(&self.inner);
            runtime.spawn(drain_watch(processor, weak, drain_limit(rto)));
            return;
        }
        self.send_destroy();
        processor.abort();
    }
}

#[cfg(test)]
#[path = "close_tests.rs"]
mod tests;
