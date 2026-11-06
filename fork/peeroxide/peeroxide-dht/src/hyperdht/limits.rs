//! Node-wide limits of the UDX stream window, applied to every stream of the node.

use libudx::UdxAsyncStream;

/// Limits applied to every UDX stream the node creates for a connection: outgoing, incoming and relayed.
/// `None` keeps the libudx default.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StreamLimits {
    /// Largest receive window in bytes (`UdxAsyncStream::set_recv_window_max`; libudx floors it at 64 KiB).
    pub recv_window_max: Option<u32>,
    /// Largest send window in bytes (`UdxAsyncStream::set_send_window_max`).
    pub send_window_max: Option<u32>,
    /// Write threshold in bytes (`UdxAsyncStream::set_write_high_watermark`).
    pub write_high_watermark: Option<usize>,
}

impl StreamLimits {
    /// Sets every knob that is `Some` on `stream`.
    pub fn apply(&self, stream: &UdxAsyncStream) {
        if let Some(bytes) = self.recv_window_max {
            stream.set_recv_window_max(bytes);
        }
        if let Some(bytes) = self.send_window_max {
            stream.set_send_window_max(bytes);
        }
        if let Some(bytes) = self.write_high_watermark {
            stream.set_write_high_watermark(bytes);
        }
    }
}

#[cfg(test)]
#[path = "limits_tests.rs"]
mod tests;
