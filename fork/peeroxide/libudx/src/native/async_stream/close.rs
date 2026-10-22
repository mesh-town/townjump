//! Close: drop before the stream ends and `abort()` send DESTROY; a finished stream closes via FIN.
// JS: udx-native/lib/stream.js

use std::net::SocketAddr;
use std::sync::Arc;

use tokio::net::UdpSocket;
use tokio::runtime::Handle;

use super::adapter::UdxAsyncStream;
use super::read::ReadState;
use crate::native::header::{FLAG_DESTROY, Header};
use crate::native::stream::{StreamInner, StreamNotify};

type DestroyPacket = (Arc<UdpSocket>, SocketAddr, Vec<u8>);

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
        if let Some((_, addr, bytes)) = destroy_packet(&inner) {
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
            let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            destroy_packet(&inner)
        };
        if let Some((udp, addr, bytes)) = packet {
            send_best_effort(udp, bytes, addr);
        }
    }
}

fn destroy_packet(inner: &StreamInner) -> Option<DestroyPacket> {
    if !inner.connected {
        return None;
    }
    let udp = inner.udp.clone()?;
    let addr = inner.remote_addr?;
    let header = Header {
        type_flags: FLAG_DESTROY,
        data_offset: 0,
        remote_id: inner.remote_id,
        recv_window: 0,
        seq: inner.next_seq,
        ack: inner.next_remote_seq,
    };
    Some((udp, addr, header.encode().to_vec()))
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

/// Three paths: stream finished — quiet processor exit; unsent data — DESTROY behind the data;
/// otherwise — direct DESTROY and `abort()` of the processor.
impl Drop for UdxAsyncStream {
    fn drop(&mut self) {
        let Some(processor) = self.processor.take() else {
            self.notify_shutdown();
            return;
        };
        if self.is_finished() {
            drop(processor);
            self.notify_shutdown();
            return;
        }
        if self.has_unsent() {
            drop(processor);
            self.queue_destroy();
            self.notify_shutdown();
            return;
        }
        self.send_destroy();
        processor.abort();
    }
}
