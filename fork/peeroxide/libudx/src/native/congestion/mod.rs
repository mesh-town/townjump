pub(crate) mod bbr;
pub(crate) mod filter;
pub(crate) mod pacing;
pub(crate) mod rate;

use crate::native::stream::diag;
use std::time::{Duration, Instant};

/// `a` is strictly after `b` in sequence space (mod 2^32, signed difference; C: `seq_compare(a, b) > 0`).
///
/// Delivery counters and packet numbers wrap at 2^32, so a plain `a > b` is wrong once either side
/// has wrapped — and wrong from the very first ACK for the initial `app_limited = u32::MAX`
/// (`~0` in C). Same arithmetic as `stream::seq::seq_after`; that one is private to `stream`.
fn seq_gt(a: u32, b: u32) -> bool {
    a.wrapping_sub(b) as i32 > 0
}

/// `a` is at or after `b` in sequence space (mod 2^32; C: `seq_compare(a, b) >= 0`).
fn seq_ge(a: u32, b: u32) -> bool {
    a.wrapping_sub(b) as i32 >= 0
}

/// BBR cycle length.
pub(crate) const BBR_CYCLE_LEN: u32 = 8;
/// Number of cycles used by the bandwidth filter.
pub(crate) const BBR_BW_FILTER_CYCLES: u32 = BBR_CYCLE_LEN + 2;
/// Minimum interval between min-RTT samples, in milliseconds.
pub(crate) const BBR_MIN_RTT_INTERVAL_MS: u64 = 10_000;
/// Minimum time spent in ProbeRTT, in milliseconds.
pub(crate) const BBR_MIN_PROBE_RTT_MODE_MS: u64 = 200;
/// Default pacing margin.
pub(crate) const BBR_PACING_MARGIN_PERCENT: f64 = 0.99;
/// Startup/high gain constant.
pub(crate) const BBR_HIGH_GAIN: f64 = 2.885_39;
/// Drain gain constant.
pub(crate) const BBR_DRAIN_GAIN: f64 = 1.0 / 2.885_39;
/// Congestion window gain constant.
pub(crate) const BBR_CWND_GAIN: f64 = 2.0;
/// Per-cycle pacing gain table.
pub(crate) const BBR_PACING_GAIN: [f64; 8] = [1.25, 0.75, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0];
/// Minimum BBR cwnd target.
pub(crate) const BBR_CWND_MIN_TARGET: u32 = 4;
/// Threshold for declaring full bandwidth.
pub(crate) const BBR_FULL_BW_THRESH: f64 = 1.25;
/// Consecutive rounds required for full bandwidth.
pub(crate) const BBR_FULL_BW_COUNT: u32 = 3;
/// Extra-acked gain.
pub(crate) const BBR_EXTRA_ACKED_GAIN: f64 = 1.0;
/// Window length for extra-acked accounting, in RTTs.
pub(crate) const BBR_EXTRA_ACKED_WIN_RTTS: u32 = 5;
/// Maximum extra-acked sample age, in milliseconds.
pub(crate) const BBR_EXTRA_ACKED_MAX_MS: u64 = 100;

/// BBR operating mode, mirroring the four phases of the BBR algorithm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BbrMode {
    /// Exponential bandwidth probing at startup.
    Startup = 0,
    /// Drain inflight back to BDP after startup overshoot.
    Drain = 1,
    /// Steady-state cycling through pacing gains.
    ProbeBw = 2,
    /// Periodic min-RTT measurement phase.
    ProbeRtt = 3,
}

/// Congestion avoidance state, tracking loss/recovery transitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd)]
pub(crate) enum CaState {
    /// Normal operation — no loss detected.
    Open = 1,
    /// Fast recovery after partial loss.
    Recovery = 2,
    /// RTO-triggered loss state.
    Loss = 3,
}

/// Congestion-control scaffold for the native libudx rewrite.
///
/// Wraps BBR state, rate sampling, and pacing into a single controller
/// that stream logic calls on send, ACK, and loss events.
pub(crate) struct CongestionController {
    /// Internal BBR algorithm state machine.
    bbr: bbr::BbrState,
    /// Delivery-rate sampling state (tracks `delivered`, timestamps, etc.).
    rate: rate::RateState,
    /// Token-bucket pacer gating outbound transmissions.
    pub(crate) pacing: pacing::TokenBucket,
    /// Cumulative count of packets acknowledged by the peer.
    pub(crate) delivered: u32,
    /// Cumulative count of packets declared lost.
    pub(crate) lost: u32,
    /// Sequence number at which the sender became application-limited (0 = not limited).
    pub(crate) app_limited: u32,
    /// Timestamp of the first packet sent in the current rate sample interval.
    pub(crate) first_sent_ts: Option<Instant>,
    /// Timestamp of the most recently delivered packet.
    pub(crate) delivered_ts: Option<Instant>,
    /// Current congestion-avoidance state (open / recovery / loss).
    pub(crate) ca_state: CaState,
    /// Highest sequence number at the time loss/recovery was entered.
    pub(crate) high_seq: u32,
    /// Current congestion window in packets.
    pub(crate) cwnd: u32,
    /// Slow-start threshold in packets.
    pub(crate) ssthresh: u32,
    /// BBR-computed pacing rate in bytes per millisecond.
    pub(crate) pacing_bytes_per_ms: u32,
    /// The stream's `udx_rtt_min` (ms; `None` — no measurements yet): the Karn protection threshold, set by `set_rtt_min`.
    rtt_min_ms: Option<u32>,
    /// Maximum segment size in bytes.
    mss: u32,
    /// Leave recovery by SACK number (`seq_ge(max_acked, high_seq)`); the F1 variant (on by default, `UDX_DIAG=nof1` turns it off) replaces it with the cumulative-ACK exit.
    pub(crate) sack_exit: bool,
}

impl CongestionController {
    /// Initial congestion window in packets — matches C libudx `udx_stream_init`.
    const INITIAL_CWND: u32 = 10;
    /// Initial slow-start threshold — matches C libudx `bbr_init` (`0xffff`).
    const INITIAL_SSTHRESH: u32 = 0xffff;

    /// Creates a congestion controller initialized with BBR state.
    pub(crate) fn new(mss: u32, _initial_rtt_ms: u32) -> Self {
        Self {
            bbr: bbr::BbrState::new(0),
            rate: rate::RateState::new(),
            pacing: pacing::TokenBucket::new(),
            delivered: 0,
            lost: 0,
            app_limited: u32::MAX, // initially app-limited (matches C `~0`)
            first_sent_ts: None,
            delivered_ts: None,
            ca_state: CaState::Open,
            high_seq: 0,
            cwnd: Self::INITIAL_CWND,
            ssthresh: Self::INITIAL_SSTHRESH,
            pacing_bytes_per_ms: 0,
            rtt_min_ms: None,
            mss,
            sack_exit: true,
        }
    }

    /// Records a packet send and returns the associated rate info.
    pub(crate) fn on_packet_sent(
        &mut self,
        _seq: u32,
        inflight: u32,
        now: Instant,
    ) -> rate::PacketRateInfo {
        if inflight == 0 || self.first_sent_ts.is_none() {
            self.first_sent_ts = Some(now);
            self.delivered_ts = Some(now);
        }
        self.rate_info(now, false)
    }

    /// Processes acknowledgements and updates BBR state.
    pub(crate) fn on_ack(
        &mut self,
        acked_packets: &[(u32, rate::PacketRateInfo)],
        lost_count: u32,
        inflight: u32,
        now: Instant,
    ) {
        let acked_count = acked_packets.len() as u32;
        self.delivered += acked_count;
        self.delivered_ts = Some(now);

        // Recovery/Loss exit: once the highest ACK'd seq reaches the recovery
        // boundary, all packets from the loss epoch have been resolved.
        if self.sack_exit && self.ca_state >= CaState::Recovery {
            // Wrap-aware (mod 2^32): a plain numeric `max`/`>=` breaks at the 2^32 rollover.
            let max_acked = acked_packets
                .iter()
                .map(|(seq, _)| *seq)
                .reduce(|a, b| if seq_gt(b, a) { b } else { a })
                .unwrap_or(0);
            if seq_ge(max_acked, self.high_seq) {
                self.ca_state = CaState::Open;
            }
        }

        // Clear app-limited once enough data has been delivered (C: udx__rate_gen). The initial
        // `u32::MAX` is already exceeded at `delivered = 1`: the comparison must be `seq_gt`.
        if self.app_limited > 0 && seq_gt(self.delivered, self.app_limited) {
            self.app_limited = 0;
        }

        // Rate sampling — clamp sub-ms RTTs to 1ms so BBR can measure on localhost.
        // RTT and send interval run from the packet's own `time_sent` (C `udx.c:1305`,
        // `udx_rate.c:49`): from `first_sent_ts` they freeze at connection start under continuous
        // sending.
        for (_seq, info) in acked_packets {
            let dur = now
                .checked_duration_since(info.time_sent)
                .unwrap_or(Duration::ZERO);
            let rtt_ms = if dur.is_zero() {
                0
            } else {
                (dur.as_millis() as u32).max(1)
            };
            // C `udx.c:1342-1344`: `rs->rtt_ms` only when `!retransmitted || rtt >= udx_rtt_min`.
            let rtt = if info.retransmitted && self.rtt_min_ms.is_none_or(|m| rtt_ms < m) {
                None
            } else {
                Some(rtt_ms)
            };
            if let Some(new_first_sent) = self.rate.on_packet_delivered(info, info.time_sent, rtt) {
                self.first_sent_ts = Some(new_first_sent);
            }
        }
        let min_rtt = self.bbr.min_rtt_ms();
        let rs = self
            .rate
            .generate(
                self.delivered,
                self.first_sent_ts.unwrap_or(now),
                self.delivered_ts.unwrap_or(now),
                min_rtt,
                lost_count,
                acked_count,
            )
            .clone();

        let cwnd_before = self.cwnd;
        // BBR main
        let (new_cwnd, new_pacing, ssthresh_out) = self.bbr.on_ack(
            &rs,
            acked_count,
            self.cwnd,
            self.mss,
            self.delivered,
            self.lost,
            inflight,
            self.ca_state,
            now,
        );
        if let Some(ssth) = ssthresh_out {
            self.ssthresh = ssth;
        }
        self.cwnd = new_cwnd;
        if new_cwnd != cwnd_before && diag::mode().z {
            let (branch, target) = self.bbr.zc_note();
            diag::cwnd_change(&diag::ZcRow {
                cwnd_before,
                cwnd_after: new_cwnd,
                branch,
                inflight_arg: i64::from(inflight),
                acked: i64::from(acked_count),
                losses: i64::from(rs.losses),
                target: i64::from(target),
                ca_state: self.ca_state as u8,
            });
        }
        self.pacing_bytes_per_ms = new_pacing;
        self.pacing.set_rate(new_pacing);

        // Mark app-limited during ProbeRtt (C: bbr_main sets app_limited after ProbeRtt)
        if self.bbr.mode() == BbrMode::ProbeRtt {
            self.app_limited = (self.delivered + inflight).max(1);
        }
        tracing::debug!(
            mode = ?self.bbr.mode(),
            cwnd = self.cwnd,
            pacing_bytes_per_ms = self.pacing_bytes_per_ms,
            delivered = self.delivered,
            inflight,
            "bbr: on_ack update"
        );
    }

    /// Records a loss event and transitions CA state.
    pub(crate) fn on_packet_lost(&mut self, _seq: u32) {
        self.lost += 1;
        if self.ca_state == CaState::Open {
            self.ca_state = CaState::Recovery;
            self.bbr.save_cwnd(self.cwnd);
            self.ssthresh = self.cwnd;
        }
    }

    /// RTO (C `udx_rto_timeout`, 1174, 1180, 1233): cwnd saved for leaving Loss, then Loss and BBR reset.
    pub(crate) fn on_rto(&mut self) {
        self.bbr.save_cwnd(self.cwnd);
        self.ca_state = CaState::Loss;
        self.bbr.on_rto();
    }

    /// C 1231: after marking on RTO the window is the packets remaining in flight plus one.
    pub(crate) fn cut_cwnd_to_flight(&mut self, inflight: u32) {
        let before = self.cwnd;
        self.cwnd = inflight + 1;
        self.diag_cwnd(before, "rto_cut", i64::from(inflight));
    }

    /// Z: the `ZC` line for a `cwnd` change outside `on_ack`; `-1` — the field is undefined in this branch.
    fn diag_cwnd(&self, before: u32, branch: &'static str, inflight: i64) {
        if self.cwnd == before || !diag::mode().z {
            return;
        }
        diag::cwnd_change(&diag::ZcRow {
            cwnd_before: before,
            cwnd_after: self.cwnd,
            branch,
            inflight_arg: inflight,
            acked: -1,
            losses: -1,
            target: -1,
            ca_state: self.ca_state as u8,
        });
    }

    /// Marks the start of a transmit opportunity.
    pub(crate) fn on_transmit_start(&mut self, now: Instant) {
        self.bbr
            .on_transmit_start(self.app_limited > 0, self.mss, now);
        self.pacing.reset(now);
    }

    /// Returns whether sending is allowed given current inflight and remote receive window.
    /// Matches C: `send_window_in_packets = min(cwnd, send_rwnd / max_payload)`, but a
    /// non-zero window below one packet still admits one packet.
    /// Read-only view of the BBR state (diagnostics: `stream::stats`).
    pub(crate) fn bbr(&self) -> &bbr::BbrState {
        &self.bbr
    }

    pub(crate) fn can_send(&self, inflight: u32, send_rwnd: u32) -> bool {
        inflight < self.send_limit(send_rwnd)
    }

    /// Right-hand side of `can_send`: the smaller of cwnd and the receiver window in packets.
    pub(crate) fn send_limit(&self, send_rwnd: u32) -> u32 {
        self.cwnd.min(rwnd_in_packets(send_rwnd, self.mss))
    }

    /// Computes pacing delay: zero while any allowance is left (the byte count does not matter,
    /// as in C `stream_may_send`), otherwise the rest of the current millisecond.
    pub(crate) fn pacing_delay(&mut self, _bytes: u32, now: Instant) -> Duration {
        self.pacing.refill(now);
        self.pacing.delay_for(now)
    }

    /// Returns whether the sender is app-limited. `queued_bytes` = send queue + sent-but-unsacked bytes (headers included),
    /// as C `writes_queued_bytes` (udx.c:1733, udx_rate.c:112-119).
    pub(crate) fn is_app_limited(
        &self,
        queued_bytes: usize,
        inflight: u32,
        retransmit_pending: bool,
    ) -> bool {
        (queued_bytes as u32) < self.mss && inflight < self.cwnd && !retransmit_pending
    }

    /// Scale congestion window after an MTU change (matches C libudx).
    pub(crate) fn on_mtu_change(&mut self, old_mtu: usize, new_mtu: usize, new_mss: usize) {
        if old_mtu > 0 {
            let before = self.cwnd;
            self.cwnd = self.cwnd * new_mtu as u32 / old_mtu as u32;
            self.diag_cwnd(before, "mtu", -1);
        }
        self.mss = new_mss as u32;
    }

    /// Update app-limited state before each packet send (C: `udx__rate_check_app_limited`).
    pub(crate) fn check_app_limited(&mut self, queued_bytes: usize, inflight: u32) {
        if self.is_app_limited(queued_bytes, inflight, false) {
            self.app_limited = (self.delivered + inflight).max(1);
        }
    }
}

/// Receiver window in packets: 0 for a zero window, otherwise at least one packet (D4).
pub(crate) fn rwnd_in_packets(send_rwnd: u32, mss: u32) -> u32 {
    if send_rwnd == 0 {
        return 0;
    }
    (send_rwnd / mss).max(1)
}

mod resend;

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "app_limited_tests.rs"]
mod app_limited_tests;

#[cfg(test)]
#[path = "rate_interval_tests.rs"]
mod rate_interval_tests;

#[cfg(test)]
#[path = "rtt_karn_tests.rs"]
mod rtt_karn_tests;
