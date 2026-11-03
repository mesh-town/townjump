// JS: libudx/src/udx.c (udx_stream_t.recv_rwnd_max, send_ack: 267-277, 915)
//! The receive window we advertise: the maximum less the bytes received and not yet read (in-order, still queued for the
//! reader, and out-of-order). Related: `receive.rs` (counting), `wire.rs` (the header), `handle.rs` and `async_stream/read.rs` (release).

use super::*;

/// Floor of the window maximum: `set_recv_window_max` never goes below it, so the window cannot be pinned shut.
const MIN_RECV_WINDOW: u32 = 65_536;

/// The window is reopened by a pure ACK when it crosses a multiple of `max / WINDOW_UPDATE_DIVISOR`.
pub(super) const WINDOW_UPDATE_DIVISOR: u32 = 4;

/// A reader's release (or a bigger maximum) is worth an ACK: the window grew across a multiple of `max / 4` and was
/// below the last step before it (a reader that keeps up never sends extra ACKs).
pub(super) fn window_update_due(old: u32, new: u32, max: u32) -> bool {
    let step = (max / WINDOW_UPDATE_DIVISOR).max(1);
    new / step > old / step && old / step < WINDOW_UPDATE_DIVISOR - 1
}

/// Writes `window` into a built packet (header bytes 8..12) just before it goes out; DESTROY keeps window 0.
pub(super) fn stamp_window(packet: &mut [u8], window: u32) {
    if packet.len() >= HEADER_SIZE && packet[2] & FLAG_DESTROY == 0 {
        packet[8..12].copy_from_slice(&window.to_le_bytes());
    }
}

impl StreamInner {
    /// Bytes received and not yet read, out-of-order included.
    pub(super) fn recv_queued(&self) -> u64 {
        (self.unread + self.recv_buf_bytes) as u64
    }

    /// The window to put into the header of an outgoing packet.
    pub(super) fn advertised_window(&self) -> u32 {
        let queued = self.recv_queued().min(u64::from(u32::MAX)) as u32;
        self.recv_rwnd_max.saturating_sub(queued)
    }

    /// `n` bytes of `Data` went to the reader's channel (under the same lock, before the send).
    pub(super) fn note_unread(&mut self, n: usize) {
        self.unread += n;
    }

    /// The reader took `n` bytes (or the channel is gone). `true` — the window grew enough to send an update.
    pub(crate) fn release_read(&mut self, n: usize) -> bool {
        let old = self.advertised_window();
        self.unread = self.unread.saturating_sub(n);
        window_update_due(old, self.advertised_window(), self.recv_rwnd_max)
    }

    /// An out-of-order packet of `n` bytes fits the window; an in-order one is always taken (a retransmission ignores the window).
    pub(super) fn ooo_fits(&self, n: usize) -> bool {
        self.recv_queued() + n as u64 <= u64::from(self.recv_rwnd_max)
    }

    /// New maximum (not below `MIN_RECV_WINDOW`). `true` — the window grew enough to send an update.
    pub(crate) fn set_recv_window_max(&mut self, bytes: u32) -> bool {
        let old = self.advertised_window();
        self.recv_rwnd_max = bytes.max(MIN_RECV_WINDOW);
        window_update_due(old, self.advertised_window(), self.recv_rwnd_max)
    }

    /// A pure ACK with the current window; nothing without a runtime (the next ACK or the probe answer carries it).
    pub(crate) fn send_window_update(&self) {
        if tokio::runtime::Handle::try_current().is_ok() {
            send_ack(self);
        }
    }
}

impl UdxStream {
    /// Sets the largest receive window this stream advertises (default 4 MiB, floor 64 KiB). Bytes received and not
    /// yet read count against it. A bigger window is announced to the peer when it grows enough.
    pub fn set_recv_window_max(&self, bytes: u32) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if inner.set_recv_window_max(bytes) {
            inner.send_window_update();
        }
    }
}

#[cfg(test)]
#[path = "recv_window_tests.rs"]
mod tests;
