//! Reading from a UDX stream; peer FIN is EOF, peer DESTROY is `ConnectionReset`.
// JS: udx-native/lib/stream.js

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Instant;

use tokio::io::{AsyncRead, ReadBuf};

use super::adapter::UdxAsyncStream;
use crate::native::stream::StreamEvent;

/// Read outcome; `Eof` and `Reset` are sticky.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum ReadState {
    Open,
    Eof,
    Reset,
}

impl UdxAsyncStream {
    /// How to tell what an `End` was: peer FIN or DESTROY. The processor sends both as `StreamEvent::End`.
    /// On DESTROY the processor sends `End` and immediately (without `await`) calls `terminate()`, on FIN it does not:
    /// `!connected` means DESTROY or a teardown. But `shutdown` + peer drop gives FIN followed by DESTROY,
    /// so on `!connected` we check whether the channel holds another `End` (then the first was FIN).
    fn classify_end(&mut self) -> ReadState {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if inner.connected {
            return ReadState::Eof;
        }
        match self.read_rx.try_recv() {
            Ok(StreamEvent::End) => ReadState::Eof,
            _ => ReadState::Reset,
        }
    }

    fn take_buffered(&mut self, buf: &mut ReadBuf<'_>) {
        let remaining = &self.read_buf[self.read_pos..];
        let n = remaining.len().min(buf.remaining());
        buf.put_slice(&remaining[..n]);
        self.read_pos += n;
        if self.read_pos >= self.read_buf.len() {
            self.read_buf.clear();
            self.read_pos = 0;
        }
    }

    fn take_data(&mut self, data: Vec<u8>, buf: &mut ReadBuf<'_>) {
        {
            let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            if inner.release_read(data.len()) {
                inner.send_window_update();
            }
        }
        let n = data.len().min(buf.remaining());
        buf.put_slice(&data[..n]);
        if n < data.len() {
            self.read_buf = data;
            self.read_pos = n;
        }
    }
}

/// Reader-side counters, logged when the adapter is dropped.
#[derive(Default)]
pub(super) struct ReadPath {
    /// Bytes `poll_read` has returned to the caller.
    taken: u64,
    last_pending: bool,
    last_poll: Option<Instant>,
}

impl UdxAsyncStream {
    /// One `debug` line with the delivered/taken counters of the read path.
    pub(super) fn log_read_stats(&self) {
        let (delivered, last_delivery) = {
            let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            (inner.read_stats.delivered, inner.read_stats.last_delivery)
        };
        let ago = |at: Option<Instant>| at.map(|t| t.elapsed().as_millis() as u64);
        tracing::debug!(
            local_id = self.local_id,
            delivered,
            taken = self.read_stats.taken,
            last_read_pending = self.read_stats.last_pending,
            read_poll_ms_ago = ?ago(self.read_stats.last_poll),
            delivery_ms_ago = ?ago(last_delivery),
            "stream read stats"
        );
    }

    fn poll_read_inner(
        &mut self,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.read_state == ReadState::Eof {
            return Poll::Ready(Ok(()));
        }
        if self.read_state == ReadState::Reset {
            return Poll::Ready(Err(reset_error()));
        }
        if self.read_pos < self.read_buf.len() {
            self.take_buffered(buf);
            return Poll::Ready(Ok(()));
        }
        match self.read_rx.poll_recv(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Some(StreamEvent::Data(data))) => {
                self.take_data(data, buf);
                Poll::Ready(Ok(()))
            }
            Poll::Ready(Some(StreamEvent::End)) => {
                self.read_state = self.classify_end();
                self.poll_read_inner(cx, buf)
            }
            Poll::Ready(None) => {
                self.read_state = ReadState::Eof;
                Poll::Ready(Ok(()))
            }
        }
    }
}

fn reset_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::ConnectionReset,
        "UDX stream destroyed by peer",
    )
}

impl AsyncRead for UdxAsyncStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = &mut *self;
        let before = buf.filled().len();
        let polled = this.poll_read_inner(cx, buf);
        this.read_stats.last_poll = Some(Instant::now());
        this.read_stats.last_pending = polled.is_pending();
        this.read_stats.taken += (buf.filled().len() - before) as u64;
        polled
    }
}

#[cfg(test)]
#[path = "read_stats_tests.rs"]
mod read_stats_tests;
