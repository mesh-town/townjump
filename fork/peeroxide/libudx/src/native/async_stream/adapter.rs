//! The `UdxAsyncStream` struct, its constructor and stats snapshot; read, write and close logic is in the sibling files.
// JS: udx-native/lib/stream.js

use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::read::ReadState;
use super::write::InFlight;
use crate::native::stream::{StreamEvent, StreamInner, WRITE_HIGH_WATERMARK};

/// Adapter that implements [`tokio::io::AsyncRead`] and [`tokio::io::AsyncWrite`]
/// over a [`crate::UdxStream`].
///
/// Created via [`crate::UdxStream::into_async_stream`]. Higher layers
/// (e.g. SecretStream) wrap this for encrypted I/O.
///
/// Writes are accepted without waiting for ACK while the unacknowledged payload is
/// within the write budget (see [`Self::WRITE_HIGH_WATER`]); `flush` waits for the
/// ACK of everything accepted.
/// Dropping the adapter before the stream has ended cleanly sends DESTROY, so the
/// peer's reader fails with `ConnectionReset` (as `stream.destroy()` does in JS).
pub struct UdxAsyncStream {
    pub(super) inner: Arc<Mutex<StreamInner>>,
    pub(super) read_rx: mpsc::UnboundedReceiver<StreamEvent>,
    pub(super) read_buf: Vec<u8>,
    pub(super) read_pos: usize,
    pub(super) read_state: ReadState,
    pub(super) in_flight: InFlight,
    pub(super) processor: Option<JoinHandle<()>>,
    pub(super) fin_queued: bool,
    pub(super) write_done: bool,
}

impl UdxAsyncStream {
    /// Sender state snapshot (RTT, window, pacing, counters) for diagnostics.
    pub fn stats(&self) -> crate::UdxStats {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.stats()
    }

    /// A handle that keeps reporting the peer's current address after the stream is split or moved.
    pub fn peer(&self) -> crate::StreamPeer {
        crate::StreamPeer(Arc::clone(&self.inner))
    }

    /// Base of the write budget, equal to C libudx `UDX_HIGH_WATERMARK` (udx.c:47).
    /// `poll_write` accepts data while the payload not yet ACKed or SACKed is below
    /// `WRITE_HIGH_WATER + min(cwnd * mss, peer receive window)`, and returns `Pending`
    /// otherwise (with any room above zero it still accepts up to 64 KiB, so a write may
    /// overshoot the budget by up to 64 KiB - 1); every ACK and SACK frees its own bytes at once. Releasing only
    /// on the cumulative ACK would cap the flight behind a head hole, so SACKed bytes are
    /// freed too, as in C.
    pub const WRITE_HIGH_WATER: usize = WRITE_HIGH_WATERMARK;

    pub(crate) fn new(
        inner: Arc<Mutex<StreamInner>>,
        read_rx: mpsc::UnboundedReceiver<StreamEvent>,
        processor: Option<JoinHandle<()>>,
    ) -> Self {
        Self {
            inner,
            read_rx,
            read_buf: Vec::new(),
            read_pos: 0,
            read_state: ReadState::Open,
            in_flight: InFlight::default(),
            processor,
            fin_queued: false,
            write_done: false,
        }
    }
}

impl Unpin for UdxAsyncStream {}
