// Node-wide knobs of the transport stream window; they apply to every stream of the node.

/// Transport stream limits applied to every stream of a node: outgoing, incoming and relayed.
/// `None` keeps the transport default.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StreamLimits {
    /// Upper bound of the receive window in bytes (libudx `set_recv_window_max`). The window advertised to
    /// the peer never exceeds it.
    pub recv_window_max: Option<u32>,
    /// Upper bound of the send window in bytes (libudx `set_send_window_max`): the most data in flight.
    pub send_window_max: Option<u32>,
    /// Write queue size in bytes above which writes report back-pressure (libudx `set_write_high_watermark`).
    pub write_high_watermark: Option<usize>,
}

#[cfg(test)]
#[path = "limits_tests.rs"]
mod tests;
