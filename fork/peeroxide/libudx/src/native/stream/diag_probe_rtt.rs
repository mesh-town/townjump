//! Instrument `ZP`, `rtt_*` and `cum_n` fields: what reaches `update_rtt` (the only writer of `srtt`). Under `UDX_DIAG=z`.
//! Only additions and comparisons, no allocations; the window is a second, reset when the line is printed.

/// RTT sample counters for the current second.
#[derive(Default)]
pub(in crate::native::stream) struct RttProbe {
    /// Receive-queue delay of the last received packet, us (set by `Zp::note_rx`; on the ACK path it is its `dq`).
    dq_cur_us: u64,
    n: u64,
    last_ms: u32,
    max_ms: u32,
    dq_max_us: u64,
    cum_n: u64,
}

impl RttProbe {
    /// Queue delay of the current packet (from `Zp::note_rx`).
    pub(in crate::native::stream) fn set_dq(&mut self, us: u64) {
        self.dq_cur_us = us;
    }

    /// A cumulative ACK advanced `remote_acked`.
    pub(in crate::native::stream) fn note_cum(&mut self) {
        self.cum_n += 1;
    }

    /// Sample `ms` passed to `update_rtt` (in ms before clamping); `dq` is taken from the ACK that gave the sample.
    pub(in crate::native::stream) fn note_sample(&mut self, ms: u32) {
        self.n += 1;
        self.last_ms = ms;
        self.max_ms = self.max_ms.max(ms);
        self.dq_max_us = self.dq_max_us.max(self.dq_cur_us);
    }

    /// Fields per window and reset; `dq_cur_us` is not reset (it describes the last packet).
    pub(in crate::native::stream) fn take(&mut self) -> String {
        let r = format!(
            "rtt_n={} rtt_last={} rtt_max={} rtt_dq_max_us={} cum_n={}",
            self.n, self.last_ms, self.max_ms, self.dq_max_us, self.cum_n
        );
        let dq = self.dq_cur_us;
        *self = Self {
            dq_cur_us: dq,
            ..Self::default()
        };
        r
    }
}
