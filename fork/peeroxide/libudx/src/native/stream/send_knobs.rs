// JS: libudx/src/udx.c (UDX_HIGH_WATERMARK, send_window_in_bytes, send_window_in_packets)
//! Per-stream sender knobs. The reference implementation has none: its write threshold is the constant
//! `UDX_HIGH_WATERMARK` and its send window is limited only by the peer window and cwnd. These are port extensions;
//! with the defaults the behavior is the same as in the reference.

use super::*;

/// Floor of `send_window_max`: a smaller cap could not be told from a closed window and would stall sending.
pub(super) const MIN_SEND_WINDOW: u32 = 4096;

impl StreamInner {
    /// The peer window as used for sending decisions: the advertised one, capped by `send_window_max`.
    /// A zero-window probe must keep looking at the real `send_rwnd`.
    pub(super) fn effective_send_rwnd(&self) -> u32 {
        self.send_rwnd.min(self.send_window_max)
    }

    pub(crate) fn set_write_high_watermark(&mut self, bytes: usize) {
        self.write_high_watermark = bytes;
    }

    pub(crate) fn set_send_window_max(&mut self, bytes: u32) {
        self.send_window_max = bytes.max(MIN_SEND_WINDOW);
    }
}

impl UdxStream {
    /// Sets how many bytes the async adapter accepts into the write queue beyond the send window (default 262144, no
    /// floor; 0 means the budget is the window alone). Port extension: the reference uses a constant.
    pub fn set_write_high_watermark(&self, bytes: usize) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.set_write_high_watermark(bytes);
    }

    /// Caps the peer window used for sending, in bytes (default `u32::MAX`, no cap; floor 4096). Bytes in flight stay
    /// within about this cap; congestion control is not changed. Port extension: the reference has no such cap.
    pub fn set_send_window_max(&self, bytes: u32) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.set_send_window_max(bytes);
    }
}

#[cfg(test)]
#[path = "send_knobs_tests.rs"]
mod tests;
