//! `AsyncRead` + `AsyncWrite` for `SecretStream`: frame-by-frame, resumable across `Pending`.
//!
//! One `poll_write` encrypts one frame (header + ciphertext in one buffer) and
//! hands it to the transport in a single write; whatever the transport did not
//! take stays in `IoState::tx` and is sent by the next `poll_*` call.
//!
//! Do not mix `read()`/`write()` with `AsyncRead`/`AsyncWrite` on one stream,
//! and do not call `into_split` after a partial `poll_*`: the buffers of
//! `IoState` are not carried over to the halves.
// JS: @hyperswarm/secret-stream/index.js

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll, ready};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use super::frame::{MAX_MESSAGE_LEN, encode_frame};
use super::session::SecretStream;
use super::timers::Timers;
use crate::secretstream::ABYTES;

/// Half-read and half-written frame state of the `AsyncRead`/`AsyncWrite` path.
#[derive(Default)]
pub(super) struct IoState {
    rx_head: [u8; 3],
    rx_head_len: usize,
    rx_body: Option<Vec<u8>>,
    rx_body_len: usize,
    plain: Vec<u8>,
    plain_pos: usize,
    tx: Vec<u8>,
    tx_pos: usize,
}

impl IoState {
    fn has_plain(&self) -> bool {
        self.plain_pos < self.plain.len()
    }

    /// Copy as much of the decrypted remainder as `buf` takes.
    fn take_plain(&mut self, buf: &mut ReadBuf<'_>) {
        let n = buf.remaining().min(self.plain.len() - self.plain_pos);
        buf.put_slice(&self.plain[self.plain_pos..self.plain_pos + n]);
        self.plain_pos += n;
        if !self.has_plain() {
            self.set_plain(Vec::new());
        }
    }

    fn set_plain(&mut self, plain: Vec<u8>) {
        self.plain = plain;
        self.plain_pos = 0;
    }

    pub(super) fn set_tx(&mut self, frame: Vec<u8>) {
        self.tx = frame;
        self.tx_pos = 0;
    }

    /// The 3-byte header is complete: size the body buffer.
    fn begin_body(&mut self) {
        let [a, b, c] = self.rx_head;
        let len = a as usize | (b as usize) << 8 | (c as usize) << 16;
        self.rx_head_len = 0;
        self.rx_body = Some(vec![0; len]);
        self.rx_body_len = 0;
    }

    /// EOF while reading the header: clean on a frame boundary, else `UnexpectedEof`.
    fn head_eof(&self) -> io::Result<Option<Vec<u8>>> {
        if self.rx_head_len == 0 {
            return Ok(None);
        }
        Err(io::ErrorKind::UnexpectedEof.into())
    }
}

/// Fill `buf[*filled..]` from `raw`; `Ok(false)` means EOF came first.
/// Every chunk that arrives counts for the idle timeout, as in JS `_onrawdata`.
fn poll_fill<R: AsyncRead + Unpin>(
    raw: &mut R,
    timers: &mut Timers,
    cx: &mut Context<'_>,
    buf: &mut [u8],
    filled: &mut usize,
) -> Poll<io::Result<bool>> {
    while *filled < buf.len() {
        let mut rb = ReadBuf::new(&mut buf[*filled..]);
        ready!(Pin::new(&mut *raw).poll_read(cx, &mut rb))?;
        if rb.filled().is_empty() {
            return Poll::Ready(Ok(false));
        }
        *filled += rb.filled().len();
        timers.rx_seen();
    }
    Poll::Ready(Ok(true))
}

fn invalid_data(e: impl std::error::Error + Send + Sync + 'static) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e)
}

/// Peer-header failure on the `AsyncRead` path: transport errors as they are, the rest `InvalidData`.
fn header_error(e: super::error::SecretStreamError) -> io::Error {
    match e {
        super::error::SecretStreamError::Io(e) => e,
        super::error::SecretStreamError::UnexpectedEof => io::ErrorKind::UnexpectedEof.into(),
        e => invalid_data(e),
    }
}

impl<T: AsyncRead + AsyncWrite + Unpin> SecretStream<T> {
    /// Next whole frame: `Ok(None)` on EOF at a frame boundary.
    pub(super) fn poll_frame(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<Option<Vec<u8>>>> {
        if self.io.rx_body.is_none() && !ready!(self.poll_head(cx))? {
            return Poll::Ready(self.io.head_eof());
        }
        self.poll_body(cx)
    }

    fn poll_head(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<bool>> {
        let io = &mut self.io;
        if !ready!(poll_fill(
            &mut self.raw,
            &mut self.timers,
            cx,
            &mut io.rx_head,
            &mut io.rx_head_len
        ))? {
            return Poll::Ready(Ok(false));
        }
        io.begin_body();
        Poll::Ready(Ok(true))
    }

    fn poll_body(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<Option<Vec<u8>>>> {
        let io = &mut self.io;
        let body = io.rx_body.get_or_insert_with(Vec::new);
        let raw = &mut self.raw;
        if !ready!(poll_fill(
            raw,
            &mut self.timers,
            cx,
            body,
            &mut io.rx_body_len
        ))? {
            return Poll::Ready(Err(io::ErrorKind::UnexpectedEof.into()));
        }
        io.rx_body_len = 0;
        Poll::Ready(Ok(io.rx_body.take()))
    }

    /// Decrypt one frame body; an empty result is a keepalive.
    fn open_frame(&mut self, frame: &[u8]) -> io::Result<Vec<u8>> {
        if frame.len() < ABYTES {
            return Err(invalid_data(
                crate::secretstream::SecretstreamError::CiphertextTooShort,
            ));
        }
        let decrypt = self
            .decrypt
            .as_mut()
            .ok_or_else(|| invalid_data(super::error::SecretStreamError::HandshakeIncomplete))?;
        let (plain, _tag) = decrypt.next(frame).map_err(invalid_data)?;
        Ok(plain)
    }

    /// Push the unsent rest of the current frame into the transport.
    pub(super) fn poll_drain_tx(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        while self.io.tx_pos < self.io.tx.len() {
            let io = &mut self.io;
            let n = ready!(Pin::new(&mut self.raw).poll_write(cx, &io.tx[io.tx_pos..]))?;
            if n == 0 {
                return Poll::Ready(Err(io::ErrorKind::WriteZero.into()));
            }
            io.tx_pos += n;
        }
        self.io.set_tx(Vec::new());
        Poll::Ready(Ok(()))
    }
}

impl<T: AsyncRead + AsyncWrite + Unpin> AsyncRead for SecretStream<T> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if buf.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        loop {
            if this.io.has_plain() {
                this.io.take_plain(buf);
                return Poll::Ready(Ok(()));
            }
            ready!(this.poll_peer_header(cx)).map_err(header_error)?;
            let Some(frame) = ready!(this.poll_frame_timed(cx))? else {
                return Poll::Ready(Ok(()));
            };
            let plain = this.open_frame(&frame)?;
            this.io.set_plain(plain);
        }
    }
}

impl<T: AsyncRead + AsyncWrite + Unpin> AsyncWrite for SecretStream<T> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        ready!(this.poll_drain_tx(cx))?;
        if buf.is_empty() {
            return Poll::Ready(Ok(0));
        }
        let n = buf.len().min(MAX_MESSAGE_LEN - ABYTES);
        let frame = encode_frame(&this.encrypt.next(&buf[..n])).map_err(io::Error::other)?;
        this.io.set_tx(frame);
        this.timers.tx_sent();
        // `Push` has already moved: the frame is ours now, its Pending/Err
        // does not change the answer (an error resurfaces on the next `poll_*`).
        let _ = this.poll_drain_tx(cx);
        Poll::Ready(Ok(n))
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        ready!(this.poll_drain_tx(cx))?;
        Pin::new(&mut this.raw).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        this.timers.stop_keep_alive();
        ready!(this.poll_drain_tx(cx))?;
        Pin::new(&mut this.raw).poll_shutdown(cx)
    }
}

#[cfg(test)]
#[path = "io_tests.rs"]
mod tests;
