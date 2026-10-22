//! BBR congestion control state machine.
//! Exact port of C libudx's `udx_bbr.c`.

use crate::native::stream::diag;
use std::time::Instant;

use super::{
    BBR_BW_FILTER_CYCLES, BBR_CWND_GAIN, BBR_CWND_MIN_TARGET, BBR_CYCLE_LEN, BBR_DRAIN_GAIN,
    BBR_EXTRA_ACKED_GAIN, BBR_EXTRA_ACKED_WIN_RTTS, BBR_FULL_BW_COUNT, BBR_FULL_BW_THRESH,
    BBR_HIGH_GAIN, BBR_MIN_PROBE_RTT_MODE_MS, BBR_MIN_RTT_INTERVAL_MS, BBR_PACING_GAIN,
    BBR_PACING_MARGIN_PERCENT, BbrMode, CaState, filter::WindowedMaxFilter, rate::RateSample,
    seq_ge,
};

/// Full BBR state machine.
///
/// Unit conventions: BW = packets/ms, CWND = packets, pacing = bytes/ms.
#[derive(Debug, Clone)]
pub(crate) struct BbrState {
    mode: BbrMode,
    prev_ca_state: CaState,
    // Bandwidth estimation
    bw_filter: WindowedMaxFilter,
    bw: f64,
    // Round tracking
    round_count: u32,
    round_start: bool,
    next_rtt_delivered: u32,
    // Startup
    full_bw: f64,
    full_bw_count: u32,
    full_bw_reached: bool,
    // PROBE_BW
    cycle_index: u32,
    cycle_stamp: Option<Instant>,
    // PROBE_RTT
    min_rtt_ms: u32,
    min_rtt_stamp: Option<Instant>,
    probe_rtt_done_stamp: Option<Instant>,
    probe_rtt_round_done: bool,
    idle_restart: bool,
    // Gains (current)
    pacing_gain: f64,
    cwnd_gain: f64,
    // Recovery
    prior_cwnd: u32,
    packet_conservation: bool,
    // ProbeRtt exit restores cwnd (C: udx_bbr.c:442-450), but cwnd is kept in mod.rs: applied on next on_ack.
    restore_cwnd: bool,
    // Ack aggregation
    extra_acked: [u32; 2],
    extra_acked_win_rtts: u32,
    extra_acked_win_rtts_thresh: u32,
    ack_epoch_acked: u32,
    ack_epoch_stamp: Option<Instant>,
    extra_acked_idx: usize,
    /// Z (`UDX_DIAG=z`, line `ZC`): branch of the last `set_cwnd` and its `target`; not written without `z`.
    zc_branch: &'static str,
    zc_target: u32,
}

impl BbrState {
    /// Initialise BBR state. Corresponds to `bbr_init()` in C.
    pub(crate) fn new(initial_rtt_ms: u32) -> Self {
        Self {
            mode: BbrMode::Startup,
            prev_ca_state: CaState::Open,
            bw_filter: WindowedMaxFilter::new(),
            bw: 0.0,
            round_count: 0,
            round_start: false,
            next_rtt_delivered: 0,
            full_bw: 0.0,
            full_bw_count: 0,
            full_bw_reached: false,
            cycle_index: 0,
            cycle_stamp: None,
            min_rtt_ms: initial_rtt_ms,
            min_rtt_stamp: Some(Instant::now()),
            probe_rtt_done_stamp: None,
            probe_rtt_round_done: false,
            idle_restart: false,
            pacing_gain: BBR_HIGH_GAIN,
            cwnd_gain: BBR_HIGH_GAIN,
            prior_cwnd: 0,
            packet_conservation: false,
            restore_cwnd: false,
            extra_acked: [0; 2],
            extra_acked_win_rtts: 0,
            extra_acked_win_rtts_thresh: BBR_EXTRA_ACKED_WIN_RTTS,
            ack_epoch_acked: 0,
            ack_epoch_stamp: None,
            extra_acked_idx: 0,
            zc_branch: "",
            zc_target: 0,
        }
    }

    /// Z: branch and `target` of the last `on_ack` (for the `ZC` line).
    pub(crate) fn zc_note(&self) -> (&'static str, u32) {
        (self.zc_branch, self.zc_target)
    }

    /// Returns the current BBR mode.
    pub(crate) fn mode(&self) -> BbrMode {
        self.mode
    }

    /// Whether the startup bandwidth plateau was detected (C `full_bw_reached`); diagnostics only.
    pub(crate) fn full_bw_reached(&self) -> bool {
        self.full_bw_reached
    }

    /// Windowed-max bandwidth estimate, packets per ms (the value pacing is derived from); diagnostics only.
    pub(crate) fn bw(&self) -> f64 {
        self.bw
    }

    /// Main BBR update on ACK. Returns `(new_cwnd, new_pacing_bytes_per_ms)`.
    /// Corresponds to `bbr_main()` in C.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn on_ack(
        &mut self,
        rs: &RateSample,
        acked: u32,
        cwnd: u32,
        mss: u32,
        delivered: u32,
        lost: u32,
        inflight: u32,
        ca_state: CaState,
        now: Instant,
    ) -> (u32, u32, Option<u32>) {
        let cwnd_in = cwnd;
        // Exit at transmit time (C: udx_bbr.c:134-148) left the restore pending.
        let cwnd = self.take_restored_cwnd(cwnd);
        let ssthresh_out = self.update_model(rs, cwnd, mss, delivered, lost, inflight, now);
        // Exit on this ACK: C restores before set_pacing_rate / set_cwnd (udx_bbr.c:532-554).
        let cwnd = self.take_restored_cwnd(cwnd);
        let z = diag::mode().z;
        let pacing = self.set_pacing_rate(self.bw, mss, self.pacing_gain);
        let new_cwnd = self.set_cwnd(
            rs,
            acked,
            cwnd,
            self.bw,
            self.cwnd_gain,
            self.min_rtt_ms,
            inflight,
            mss,
            ca_state,
            delivered,
        );
        // C caps ProbeRtt after the `done:` label, on every set_cwnd path (udx_bbr.c:247-249).
        let capped = self.probe_rtt_cwnd(new_cwnd);
        if z {
            self.zc_override(cwnd_in != cwnd, capped != new_cwnd);
        }
        (capped, pacing, ssthresh_out)
    }

    /// Handle RTO event. Corresponds to `bbr_on_rto()` in C.
    pub(crate) fn on_rto(&mut self) {
        // C udx_bbr.c:609-615: `prev_ca_state = UDX_CA_LOSS; full_bw = 0.0; round_start = true;`.
        // full_bw_count and full_bw_reached survive, so a ProbeRtt exit after a timeout still returns to ProbeBw.
        self.full_bw = 0.0;
        self.round_start = true;
        self.prev_ca_state = CaState::Loss;
    }

    /// Handle start of a transmit opportunity.
    pub(crate) fn on_transmit_start(&mut self, app_limited: bool, _mss: u32, now: Instant) {
        if !app_limited {
            return;
        }
        self.idle_restart = true;
        self.ack_epoch_stamp = Some(now);
        self.ack_epoch_acked = 0;
        // C: udx_bbr.c:134-148 — in ProbeRtt only the dwell timer gates this exit.
        self.check_probe_rtt_done(now);
    }

    /// Returns the cwnd to save (used as ssthresh). Corresponds to `bbr_save_cwnd()` in C.
    pub(crate) fn save_cwnd(&mut self, cwnd: u32) {
        // A loss before the pending restore lands must see the cwnd C already restored.
        let cwnd = self.restored_cwnd(cwnd);
        if self.prev_ca_state < CaState::Recovery && self.mode != BbrMode::ProbeRtt {
            self.prior_cwnd = cwnd;
        } else {
            self.prior_cwnd = self.prior_cwnd.max(cwnd);
        }
    }

    /// Returns the current minimum RTT in ms; never returns 0.
    pub(crate) fn min_rtt_ms(&self) -> u32 {
        if self.min_rtt_ms == 0 {
            1
        } else {
            self.min_rtt_ms
        }
    }
}

// ─── Private update pipeline ─────────────────────────────────────────────────

impl BbrState {
    /// Full model update. Inner body of `bbr_main()`.
    #[allow(clippy::too_many_arguments)]
    fn update_model(
        &mut self,
        rs: &RateSample,
        cwnd: u32,
        mss: u32,
        delivered: u32,
        _lost: u32,
        inflight: u32,
        now: Instant,
    ) -> Option<u32> {
        self.update_bw(rs, delivered);
        self.update_ack_aggregation(rs, cwnd, now);
        self.update_cycle_phase(rs, inflight, self.min_rtt_ms, now);
        self.check_full_bw_reached(rs);
        let ssthresh_out = self.check_drain(inflight, mss, now);
        self.update_min_rtt(rs, cwnd, delivered, inflight, now);
        self.update_gains();
        ssthresh_out
    }

    /// Update bandwidth estimate and round counter.
    fn update_bw(&mut self, rs: &RateSample, delivered: u32) {
        self.round_start = false;

        if rs.delivered == 0 || rs.interval_ms == 0 {
            return;
        }

        // Round detection: a new round starts when we ACK past the RTT boundary
        // (C: udx_bbr.c:320-335, after the early return). The counters wrap at 2^32, and the
        // reference compares with `seq_diff(...) >= 0` (udx_bbr.c:330), so a plain `>=` would
        // miss every round boundary for a whole wrap once `delivered` passes u32::MAX.
        if seq_ge(rs.prior_delivered, self.next_rtt_delivered) {
            self.next_rtt_delivered = delivered;
            self.round_count = self.round_count.wrapping_add(1);
            self.round_start = true;
            // Packet conservation lasts one round (C: udx_bbr.c:334).
            self.packet_conservation = false;
        }

        let delivery_rate = rs.delivered as f64 / rs.interval_ms as f64;
        // Only update filter if not app-limited, or if the new rate reaches the max (C: udx_bbr.c:341).
        if !rs.is_app_limited || delivery_rate >= self.bw_filter.get() {
            self.bw_filter
                .update(delivery_rate, self.round_count, BBR_BW_FILTER_CYCLES);
            self.bw = self.bw_filter.get();
        }
    }

    /// Update ack aggregation accounting.
    fn update_ack_aggregation(&mut self, rs: &RateSample, cwnd: u32, now: Instant) {
        if rs.acked_sacked == 0 || rs.delivered == 0 || rs.interval_ms == 0 {
            return;
        }

        // On new round: advance window slot if threshold reached.
        if self.round_start {
            self.extra_acked_win_rtts = self.extra_acked_win_rtts.saturating_add(1).min(0xff);
            if self.extra_acked_win_rtts >= self.extra_acked_win_rtts_thresh {
                self.extra_acked_win_rtts = 0;
                // Rotate: discard old window, start fresh slot.
                self.extra_acked_idx ^= 1;
                self.extra_acked[self.extra_acked_idx] = 0;
            }
        }

        // Initialise epoch on first call.
        if self.ack_epoch_stamp.is_none() {
            self.ack_epoch_stamp = Some(now);
            self.ack_epoch_acked = 0;
        }

        let elapsed_ms = self
            .ack_epoch_stamp
            .and_then(|stamp| now.checked_duration_since(stamp))
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let mut expected_acked = (elapsed_ms as f64 * self.bw) as u32;

        if self.ack_epoch_acked <= expected_acked
            || (self.ack_epoch_acked as u64).saturating_add(rs.acked_sacked as u64)
                >= (u32::MAX as u64)
        /* bbr_ack_epoch_acked_reset_thresh = UINT32_MAX in C */
        {
            self.ack_epoch_acked = 0;
            self.ack_epoch_stamp = Some(now);
            expected_acked = 0;
        }

        self.ack_epoch_acked = self.ack_epoch_acked.saturating_add(rs.acked_sacked);
        let extra_acked = self
            .ack_epoch_acked
            .saturating_sub(expected_acked)
            .min(cwnd);

        if extra_acked > self.extra_acked[self.extra_acked_idx] {
            self.extra_acked[self.extra_acked_idx] = extra_acked;
        }
    }

    /// Advance the ProbeBw cycle phase when conditions are met.
    fn update_cycle_phase(
        &mut self,
        rs: &RateSample,
        inflight: u32,
        min_rtt_ms: u32,
        now: Instant,
    ) {
        if self.mode != BbrMode::ProbeBw {
            return;
        }

        let elapsed_enough = self
            .cycle_stamp
            .and_then(|stamp| now.checked_duration_since(stamp))
            .map(|d| d.as_millis() as u64 > min_rtt_ms as u64)
            .unwrap_or(true);

        if self.pacing_gain == 1.0 {
            if elapsed_enough {
                self.advance_cycle_phase(now);
            }
            return;
        }

        if self.pacing_gain > 1.0 {
            let target = self.target_cwnd(self.bw, self.pacing_gain, min_rtt_ms, 0);
            if elapsed_enough && (rs.losses > 0 || inflight > target) {
                self.advance_cycle_phase(now);
            }
            return;
        }

        let target = self.target_cwnd(self.bw, 1.0, min_rtt_ms, 0);
        if elapsed_enough || inflight <= target {
            self.advance_cycle_phase(now);
        }
    }

    /// Check whether full bandwidth has been reached (Startup only).
    fn check_full_bw_reached(&mut self, rs: &RateSample) {
        if self.full_bw_reached || !self.round_start || rs.is_app_limited {
            return;
        }
        if self.bw >= self.full_bw * BBR_FULL_BW_THRESH {
            // BW grew by threshold: reset baseline.
            self.full_bw = self.bw;
            self.full_bw_count = 0;
            return;
        }
        self.full_bw_count += 1;
        self.full_bw_reached = self.full_bw_count >= BBR_FULL_BW_COUNT;
    }

    /// Transition from Drain to ProbeBw once the queue has drained.
    fn check_drain(&mut self, inflight: u32, mss: u32, now: Instant) -> Option<u32> {
        let mut ssthresh_out = None;
        if self.mode == BbrMode::Startup && self.full_bw_reached {
            tracing::debug!(from = ?BbrMode::Startup, to = ?BbrMode::Drain, "bbr: mode transition");
            self.mode = BbrMode::Drain;
            ssthresh_out = Some(self.target_cwnd(self.bw, 1.0, self.min_rtt_ms, mss));
        }
        if self.mode == BbrMode::Drain {
            let target = self.target_cwnd(self.bw, 1.0, self.min_rtt_ms, mss);
            if inflight <= target {
                self.reset_probe_bw_mode(now);
            }
        }
        ssthresh_out
    }

    /// Update minimum RTT estimate and handle ProbeRtt entry / exit.
    fn update_min_rtt(
        &mut self,
        rs: &RateSample,
        cwnd: u32,
        delivered: u32,
        inflight: u32,
        now: Instant,
    ) {
        let min_rtt_expired = self
            .min_rtt_stamp
            .and_then(|stamp| now.checked_duration_since(stamp))
            .map(|d| d.as_millis() as u64 > BBR_MIN_RTT_INTERVAL_MS)
            .unwrap_or(false);

        if rs.rtt_ms > 0 && (self.min_rtt_ms == 0 || rs.rtt_ms < self.min_rtt_ms || min_rtt_expired)
        {
            self.min_rtt_ms = rs.rtt_ms;
            self.min_rtt_stamp = Some(now);
        }

        // Enter ProbeRtt if the min-RTT sample has gone stale.
        if min_rtt_expired && !self.idle_restart && self.mode != BbrMode::ProbeRtt {
            tracing::debug!(from = ?self.mode, to = ?BbrMode::ProbeRtt, "bbr: mode transition");
            self.mode = BbrMode::ProbeRtt;
            self.save_cwnd(cwnd);
            self.probe_rtt_done_stamp = None;
            self.probe_rtt_round_done = false;
        }

        self.update_probe_rtt(delivered, inflight, now);

        if rs.delivered > 0 {
            self.idle_restart = false;
        }
    }

    /// ProbeRtt dwell: arm the timer, mark the round, try to exit. C: udx_bbr.c:486-500.
    fn update_probe_rtt(&mut self, delivered: u32, inflight: u32, now: Instant) {
        if self.mode != BbrMode::ProbeRtt {
            return;
        }
        if self.probe_rtt_done_stamp.is_none() {
            self.arm_probe_rtt(delivered, inflight, now);
            return;
        }
        // An armed timer survives inflight above the floor (C has no inflight check here).
        if !(self.probe_rtt_round_done || self.round_start) {
            return;
        }
        self.probe_rtt_round_done = true;
        self.check_probe_rtt_done(now);
    }

    /// Arm the dwell timer once inflight is down to the floor. C: udx_bbr.c:490-493.
    fn arm_probe_rtt(&mut self, delivered: u32, inflight: u32, now: Instant) {
        if inflight > BBR_CWND_MIN_TARGET {
            return;
        }
        self.probe_rtt_done_stamp = Some(now);
        self.probe_rtt_round_done = false;
        // The dwell must also span a full round that starts here.
        self.next_rtt_delivered = delivered;
    }

    /// Leave ProbeRtt once the dwell is over. C: udx_bbr.c:442-450.
    fn check_probe_rtt_done(&mut self, now: Instant) {
        if self.mode != BbrMode::ProbeRtt || !self.probe_rtt_dwell_done(now) {
            return;
        }
        self.min_rtt_stamp = Some(now);
        self.restore_cwnd = true;
        self.reset_mode(now);
    }

    fn probe_rtt_dwell_done(&self, now: Instant) -> bool {
        self.probe_rtt_done_stamp
            .and_then(|stamp| now.checked_duration_since(stamp))
            .map(|d| d.as_millis() as u64 > BBR_MIN_PROBE_RTT_MODE_MS)
            .unwrap_or(false)
    }

    /// Mode after ProbeRtt follows the full-bandwidth flag. C: udx_bbr.c:313-319.
    fn reset_mode(&mut self, now: Instant) {
        if self.full_bw_reached {
            self.reset_probe_bw_mode(now);
            return;
        }
        self.reset_startup_mode();
    }

    /// C: udx_bbr.c:289-294 — only the mode; gains follow in update_gains.
    fn reset_startup_mode(&mut self) {
        tracing::debug!(from = ?self.mode, to = ?BbrMode::Startup, "bbr: mode transition");
        self.mode = BbrMode::Startup;
    }

    /// C: udx_bbr.c:296-310.
    fn reset_probe_bw_mode(&mut self, now: Instant) {
        tracing::debug!(from = ?self.mode, to = ?BbrMode::ProbeBw, "bbr: mode transition");
        self.mode = BbrMode::ProbeBw;
        self.cycle_index = 3;
        self.advance_cycle_phase(now); // → cycle_index becomes 4
    }

    /// Refresh `pacing_gain` / `cwnd_gain` from the current mode.
    fn update_gains(&mut self) {
        match self.mode {
            BbrMode::Startup => {
                self.pacing_gain = BBR_HIGH_GAIN;
                self.cwnd_gain = BBR_HIGH_GAIN;
            }
            BbrMode::Drain => {
                self.pacing_gain = BBR_DRAIN_GAIN;
                self.cwnd_gain = BBR_HIGH_GAIN;
            }
            BbrMode::ProbeBw => {
                self.pacing_gain = BBR_PACING_GAIN[self.cycle_index as usize];
                self.cwnd_gain = BBR_CWND_GAIN;
            }
            BbrMode::ProbeRtt => {
                self.pacing_gain = 1.0;
                self.cwnd_gain = 1.0;
            }
        }
    }

    /// Compute target pacing rate (bytes/ms).
    fn set_pacing_rate(&self, bw: f64, mss: u32, pacing_gain: f64) -> u32 {
        (bw * mss as f64 * pacing_gain * BBR_PACING_MARGIN_PERCENT) as u32
    }

    /// Compute new congestion window (packets).
    #[allow(clippy::too_many_arguments)]
    fn set_cwnd(
        &mut self,
        rs: &RateSample,
        acked: u32,
        cwnd: u32,
        bw: f64,
        cwnd_gain: f64,
        min_rtt_ms: u32,
        inflight: u32,
        mss: u32,
        ca_state: CaState,
        delivered: u32,
    ) -> u32 {
        let z = diag::mode().z;
        if z {
            (self.zc_branch, self.zc_target) = ("", 0);
        }
        let mut new_cwnd = cwnd;
        if rs.losses > 0 {
            new_cwnd = new_cwnd.saturating_sub(rs.losses).max(1);
        }

        if ca_state == CaState::Recovery && self.prev_ca_state != CaState::Recovery {
            self.packet_conservation = true;
            self.next_rtt_delivered = delivered;
            new_cwnd = inflight.saturating_add(acked);
            self.zc_set(z, "enter");
        } else if self.prev_ca_state >= CaState::Recovery && ca_state == CaState::Open {
            new_cwnd = new_cwnd.max(self.prior_cwnd);
            self.packet_conservation = false;
            self.zc_set(z, "restore");
        }

        self.prev_ca_state = ca_state;

        if self.packet_conservation {
            self.zc_set(z, "conservation");
            return new_cwnd
                .max(inflight.saturating_add(acked))
                .max(BBR_CWND_MIN_TARGET);
        }

        if rs.delivered == 0 {
            self.zc_set(z, "delivered0");
            return new_cwnd.max(BBR_CWND_MIN_TARGET);
        }

        let target = self
            .target_cwnd(bw, cwnd_gain, min_rtt_ms, mss)
            .saturating_add(self.extra_acked());
        if z {
            self.zc_target = target;
        }
        self.zc_set(
            z,
            if self.full_bw_reached {
                "full_bw"
            } else {
                "grow"
            },
        );

        if self.full_bw_reached {
            // Full BW reached: grow up to BDP-based target (matches C bbr_full_bw_reached()).
            new_cwnd = new_cwnd.saturating_add(acked).min(target);
        } else {
            // Still in Startup: grow freely.
            new_cwnd = new_cwnd.saturating_add(acked);
        }

        // Always enforce the floor.
        new_cwnd.max(BBR_CWND_MIN_TARGET)
    }

    /// Z: `set_cwnd` branch; the first one set per call stays (`enter`/`restore` outrank `conservation`).
    fn zc_set(&mut self, z: bool, branch: &'static str) {
        if z && self.zc_branch.is_empty() {
            self.zc_branch = branch;
        }
    }

    /// Z: branches outside `set_cwnd` — deferred restore after ProbeRtt and the ProbeRtt cut.
    fn zc_override(&mut self, restored: bool, capped: bool) {
        if capped {
            self.zc_branch = "probe_rtt";
        } else if restored && self.zc_branch != "enter" {
            self.zc_branch = "restore";
        }
    }

    /// Advance to the next ProbeBw cycle slot.
    fn advance_cycle_phase(&mut self, now: Instant) {
        self.cycle_stamp = Some(now);
        self.cycle_index = (self.cycle_index + 1) % BBR_CYCLE_LEN;
    }

    /// Compute target cwnd (packets) as BDP × gain.
    fn target_cwnd(&self, bw: f64, gain: f64, min_rtt_ms: u32, _mss: u32) -> u32 {
        let bdp = (bw * min_rtt_ms as f64) as u32;
        let cwnd = (bdp as f64 * gain) as u32;
        cwnd.max(BBR_CWND_MIN_TARGET)
    }

    /// Current extra-acked estimate (packets), scaled by `BBR_EXTRA_ACKED_GAIN`.
    fn extra_acked(&self) -> u32 {
        let max_extra = self.extra_acked[0].max(self.extra_acked[1]);
        (max_extra as f64 * BBR_EXTRA_ACKED_GAIN) as u32
    }

    /// `cwnd` with the pending ProbeRtt-exit restore applied; unchanged when none is pending.
    fn restored_cwnd(&self, cwnd: u32) -> u32 {
        if !self.restore_cwnd {
            return cwnd;
        }
        cwnd.max(self.prior_cwnd)
    }

    /// Apply the pending restore once and clear it.
    fn take_restored_cwnd(&mut self, cwnd: u32) -> u32 {
        let restored = self.restored_cwnd(cwnd);
        self.restore_cwnd = false;
        restored
    }

    /// ProbeRtt holds cwnd at the floor. C: udx_bbr.c:247-249.
    fn probe_rtt_cwnd(&self, cwnd: u32) -> u32 {
        if self.mode != BbrMode::ProbeRtt {
            return cwnd;
        }
        cwnd.min(BBR_CWND_MIN_TARGET)
    }
}

#[cfg(test)]
#[path = "bbr_tests.rs"]
mod tests;
