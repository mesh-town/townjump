//! Reading from a UDX stream; peer FIN is EOF, peer DESTROY is `ConnectionReset`.
// JS: udx-native/lib/stream.js

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

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
        let n = data.len().min(buf.remaining());
        buf.put_slice(&data[..n]);
        if n < data.len() {
            self.read_buf = data;
            self.read_pos = n;
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
        if this.read_state == ReadState::Eof {
            return Poll::Ready(Ok(()));
        }
        if this.read_state == ReadState::Reset {
            return Poll::Ready(Err(reset_error()));
        }
        if this.read_pos < this.read_buf.len() {
            this.take_buffered(buf);
            return Poll::Ready(Ok(()));
        }
        match this.read_rx.poll_recv(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Some(StreamEvent::Data(data))) => {
                this.take_data(data, buf);
                Poll::Ready(Ok(()))
            }
            Poll::Ready(Some(StreamEvent::End)) => {
                this.read_state = this.classify_end();
                Pin::new(this).poll_read(cx, buf)
            }
            Poll::Ready(None) => {
                this.read_state = ReadState::Eof;
                Poll::Ready(Ok(()))
            }
        }
    }
}
