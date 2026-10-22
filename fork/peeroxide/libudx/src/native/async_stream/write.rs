//! Write to a UDX stream: accepts up to the budget `WRITE_HIGH_WATER + min(cwnd·mss, rwnd)` as in C; `flush` and `shutdown` wait for ACKs.
// JS: udx-native/lib/stream.js

use std::collections::VecDeque;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll, Waker};
use std::time::Instant;

use tokio::io::AsyncWrite;
use tokio::sync::oneshot;

use super::adapter::UdxAsyncStream;
use crate::error::Result as UdxResult;
use crate::native::header::{FLAG_END, Header};
use crate::native::stream::{DEFAULT_RWND, PendingWrite, StreamInner, build_data_packet};

type AckRx = oneshot::Receiver<UdxResult<()>>;

/// Accept floor when headroom > 0: a write of up to 64 KiB is accepted whole, as in C (udx.c:2744).
const WRITE_CHUNK: usize = 64 * 1024;

/// Accepted writes still awaiting an ACK, in send order: ACK receivers (FIN included).
#[derive(Default)]
pub(super) struct InFlight {
    acks: VecDeque<AckRx>,
}

impl InFlight {
    pub(super) fn push(&mut self, rx: AckRx) {
        self.acks.push_back(rx);
    }

    /// Pops ready writes off the head; `Pending` means the head has no ACK yet (waker is on it).
    /// A ready write is popped at once: polling a ready `oneshot::Receiver` again panics.
    pub(super) fn poll_settle(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        loop {
            let Some(rx) = self.acks.front_mut() else {
                return Poll::Ready(Ok(()));
            };
            let result = match Pin::new(rx).poll(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(result) => result,
            };
            self.acks.pop_front();
            match result {
                Ok(Ok(())) => {}
                Ok(Err(e)) => return Poll::Ready(Err(io::Error::other(e.to_string()))),
                Err(_) => return Poll::Ready(Err(closed_error())),
            }
        }
    }
}

fn closed_error() -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, "UDX stream closed")
}

/// Puts data on the send queue: `prepare_write`, slicing by `max_payload`, packets.
/// Returns the ACK receiver for the whole write. The caller holds the lock: MTU does not change between steps.
fn queue_data(guard: &mut StreamInner, data: &[u8]) -> io::Result<AckRx> {
    let prep = guard
        .prepare_write(data.len())
        .map_err(|e| io::Error::other(e.to_string()))?;
    let max_payload = guard.max_payload();
    let packets: Vec<(u32, Vec<u8>)> = data
        .chunks(max_payload)
        .enumerate()
        .map(|(i, chunk)| {
            let seq = prep.first_seq.wrapping_add(i as u32);
            (
                seq,
                build_data_packet(prep.remote_id, seq, prep.current_ack, chunk),
            )
        })
        .collect();
    if guard.send_idle() {
        guard.congestion.on_transmit_start(Instant::now());
    }
    guard.queue_for_send(packets, prep.remote_addr);
    Ok(prep.ack_rx)
}

impl UdxAsyncStream {
    /// Accepts a write under a single lock hold: rejections, headroom, parking or queueing.
    /// Headroom 0 gives `Pending` (writer waker registered); otherwise accepts `min(buf, max(headroom, WRITE_CHUNK))`.
    fn accept(&self, buf: &[u8], waker: &Waker) -> Poll<io::Result<(AckRx, usize)>> {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Err(e) = guard.check_writable() {
            return Poll::Ready(Err(io::Error::other(e.to_string())));
        }
        let room = guard.write_room();
        if room == 0 {
            guard.park_writer(waker);
            return Poll::Pending;
        }
        let n = buf.len().min(room.max(WRITE_CHUNK));
        Poll::Ready(queue_data(&mut guard, &buf[..n]).map(|rx| (rx, n)))
    }

    /// Queues FIN behind the data; `None` means no peer address, only data goes out.
    fn queue_fin(&mut self) -> Option<AckRx> {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let addr = inner.remote_addr?;
        let seq = inner.reserve_end_seq();
        let header = Header {
            type_flags: FLAG_END,
            data_offset: 0,
            remote_id: inner.remote_id,
            recv_window: DEFAULT_RWND,
            seq,
            ack: inner.next_remote_seq,
        };
        let (ack_tx, ack_rx) = oneshot::channel();
        inner.pending_writes.push(PendingWrite {
            ack_threshold: seq.wrapping_add(1),
            tx: Some(ack_tx),
        });
        inner.queue_for_send(vec![(seq, header.encode().to_vec())], addr);
        Some(ack_rx)
    }

    fn is_connected(&self) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .connected
    }
}

impl AsyncWrite for UdxAsyncStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = &mut *self;
        if this.fin_queued {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "UDX stream shut down",
            )));
        }
        if let Poll::Ready(Err(e)) = this.in_flight.poll_settle(cx) {
            return Poll::Ready(Err(e));
        }
        if buf.is_empty() {
            return Poll::Ready(Ok(0));
        }
        let (rx, n) = match this.accept(buf, cx.waker()) {
            Poll::Ready(Ok(accepted)) => accepted,
            Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
            Poll::Pending => return Poll::Pending,
        };
        this.in_flight.push(rx);
        Poll::Ready(Ok(n))
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.in_flight.poll_settle(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = &mut *self;
        if !this.fin_queued {
            if !this.is_connected() {
                return Poll::Ready(Ok(()));
            }
            this.fin_queued = true;
            if let Some(rx) = this.queue_fin() {
                this.in_flight.push(rx);
            }
        }
        match this.in_flight.poll_settle(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Ok(())) => {
                this.write_done = true;
                Poll::Ready(Ok(()))
            }
            Poll::Ready(Err(e)) if e.kind() == io::ErrorKind::BrokenPipe => Poll::Ready(Ok(())),
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
        }
    }
}

#[cfg(test)]
#[path = "write_tests.rs"]
mod tests;
