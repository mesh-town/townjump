//! Tests for `bbr.rs`; parity with C libudx `udx_bbr.c`.

use super::*;
use std::time::{Duration, Instant};

const MSS: u32 = 1180;

fn at(t0: Instant, offset_ms: u64) -> Instant {
    t0 + Duration::from_millis(offset_ms)
}

/// Rate sample; `acked_sacked` equals `delivered`.
fn sample(
    delivered: u32,
    interval_ms: u32,
    rtt_ms: u32,
    prior_delivered: u32,
    app_limited: bool,
) -> RateSample {
    let mut rs = RateSample::default();
    rs.delivered = delivered;
    rs.interval_ms = interval_ms;
    rs.rtt_ms = rtt_ms;
    rs.acked_sacked = delivered;
    rs.prior_delivered = prior_delivered;
    rs.is_app_limited = app_limited;
    rs
}

/// Filled pipe: ProbeBw at gain 1.0, bw 1 packet/ms, min_rtt 250 ms stamped at `t0`.
fn full_pipe(t0: Instant) -> BbrState {
    let mut b = BbrState::new(250);
    b.mode = BbrMode::ProbeBw;
    b.cycle_index = 4;
    b.pacing_gain = 1.0;
    b.cwnd_gain = BBR_CWND_GAIN;
    b.full_bw_reached = true;
    b.full_bw_count = BBR_FULL_BW_COUNT;
    b.bw_filter.update(1.0, 0, BBR_BW_FILTER_CYCLES);
    b.bw = 1.0;
    b.min_rtt_stamp = Some(t0);
    b
}

/// ProbeRtt with the dwell timer armed at `t0`; the round ends at 100 delivered packets.
fn in_probe_rtt(t0: Instant, prior_cwnd: u32) -> BbrState {
    let mut b = full_pipe(t0);
    b.mode = BbrMode::ProbeRtt;
    b.prior_cwnd = prior_cwnd;
    b.probe_rtt_done_stamp = Some(t0);
    b.next_rtt_delivered = 100;
    b.pacing_gain = 1.0;
    b.cwnd_gain = 1.0;
    b
}

/// One ACK in CA Open without losses, `acked = rs.delivered`; returns the new cwnd.
fn ack(
    b: &mut BbrState,
    rs: &RateSample,
    cwnd: u32,
    delivered: u32,
    inflight: u32,
    now: Instant,
) -> u32 {
    let (new_cwnd, _, _) = b.on_ack(
        rs,
        rs.delivered,
        cwnd,
        MSS,
        delivered,
        0,
        inflight,
        CaState::Open,
        now,
    );
    new_cwnd
}

mod probe_rtt_exit {
    use super::*;

    #[test]
    fn ack_after_dwell_and_round_restores_prior_cwnd_and_enters_probe_bw() {
        // C udx_bbr.c:442-450: cwnd = max(cwnd, prior_cwnd), then bbr_reset_mode.
        let t0 = Instant::now();
        let t1 = at(t0, 250);
        let mut b = in_probe_rtt(t0, 100);
        let cwnd = ack(&mut b, &sample(4, 250, 250, 100, true), 4, 104, 0, t1);
        assert_eq!(
            cwnd, 104,
            "max(4, 100) + 4 acked, under the 2 x BDP target of 500"
        );
        assert_eq!(b.mode, BbrMode::ProbeBw);
        assert_eq!(b.cycle_index, 4);
        assert_eq!(b.cycle_stamp, Some(t1));
        assert_eq!(b.min_rtt_stamp, Some(t1));
        assert_eq!((b.pacing_gain, b.cwnd_gain), (1.0, BBR_CWND_GAIN));

        let mut b = in_probe_rtt(t0, 20);
        let cwnd = ack(&mut b, &sample(4, 250, 250, 100, true), 30, 104, 0, t1);
        assert_eq!(cwnd, 34, "max never lowers a cwnd above prior_cwnd");
    }

    #[test]
    fn exit_after_rto_returns_to_probe_bw_not_startup() {
        // C bbr_on_rto (udx_bbr.c:609-615) keeps bbr_full_bw_reached, so bbr_reset_mode picks ProbeBw.
        let t0 = Instant::now();
        let mut b = in_probe_rtt(t0, 100);
        b.on_rto();
        assert!(b.full_bw_reached);
        ack(
            &mut b,
            &sample(4, 250, 250, 100, true),
            4,
            104,
            0,
            at(t0, 250),
        );
        assert_eq!(b.mode, BbrMode::ProbeBw);
    }

    #[test]
    fn exit_before_full_bw_reached_returns_to_startup_whatever_the_counter() {
        // C udx_bbr.c:313-319: the mode follows bbr_full_bw_reached, not full_bw_count.
        let t0 = Instant::now();
        for count in [1, 2] {
            let mut b = in_probe_rtt(t0, 100);
            b.full_bw_reached = false;
            b.full_bw_count = count;
            let cwnd = ack(
                &mut b,
                &sample(4, 250, 250, 100, true),
                4,
                104,
                0,
                at(t0, 250),
            );
            assert_eq!(b.mode, BbrMode::Startup, "full_bw_count {count}");
            assert_eq!(b.pacing_gain, BBR_HIGH_GAIN);
            assert_eq!(b.cycle_index, 4, "Startup reset leaves the cycle alone");
            assert_eq!(cwnd, 104);
        }
    }

    #[test]
    fn entry_saves_max_of_prior_and_cwnd_and_exit_restores_it() {
        // C udx_bbr.c:481-482 sets the mode before bbr_save_cwnd, whose ProbeRtt branch keeps the max.
        for (prior, restored) in [(120, 124), (0, 84)] {
            let t0 = Instant::now();
            let t1 = at(t0, 10_500);
            let mut b = full_pipe(t0);
            b.prior_cwnd = prior;
            let cwnd = ack(&mut b, &sample(4, 250, 250, 196, true), 80, 200, 3, t1);
            assert_eq!(b.mode, BbrMode::ProbeRtt);
            assert_eq!(cwnd, BBR_CWND_MIN_TARGET);
            let cwnd = ack(
                &mut b,
                &sample(4, 250, 250, 200, true),
                cwnd,
                204,
                0,
                at(t1, 250),
            );
            assert_eq!(b.mode, BbrMode::ProbeBw, "prior {prior}");
            assert_eq!(cwnd, restored, "prior {prior}");
        }
    }
}

mod probe_rtt_dwell {
    use super::*;

    #[test]
    fn stale_min_rtt_stamp_is_not_refreshed_inside_probe_rtt() {
        // C udx_bbr.c:472-476 writes min_rtt_stamp only for a taken sample, 447 on exit.
        let t0 = Instant::now();
        let mut b = full_pipe(t0);
        ack(
            &mut b,
            &sample(4, 250, 0, 196, true),
            40,
            200,
            10,
            at(t0, 10_500),
        );
        assert_eq!(b.mode, BbrMode::ProbeRtt);
        assert_eq!(
            b.probe_rtt_done_stamp, None,
            "inflight 10 > 4 keeps the timer unarmed"
        );
        let t2 = at(t0, 10_550);
        ack(&mut b, &sample(4, 250, 300, 200, true), 4, 204, 10, t2);
        assert_eq!(b.min_rtt_ms, 300, "the expired filter takes any sample");
        assert_eq!(b.min_rtt_stamp, Some(t2));
    }

    #[test]
    fn armed_timer_survives_inflight_above_floor_and_exits() {
        // C udx_bbr.c:494-499: the `else if (probe_rtt_done_time)` branch has no inflight check.
        let t0 = Instant::now();
        let mut b = in_probe_rtt(t0, 100);
        ack(
            &mut b,
            &sample(4, 250, 250, 50, true),
            6,
            54,
            6,
            at(t0, 100),
        );
        assert_eq!(b.probe_rtt_done_stamp, Some(t0));
        assert_eq!(b.mode, BbrMode::ProbeRtt);
        ack(
            &mut b,
            &sample(4, 250, 250, 100, true),
            6,
            104,
            6,
            at(t0, 250),
        );
        assert_eq!(b.mode, BbrMode::ProbeBw);
    }

    #[test]
    fn arming_the_timer_starts_a_new_round() {
        // C udx_bbr.c:490-493: next_rtt_delivered = delivered when the timer is armed.
        let t0 = Instant::now();
        let t1 = at(t0, 10);
        let mut b = in_probe_rtt(t0, 100);
        b.probe_rtt_done_stamp = None;
        b.next_rtt_delivered = 50;
        ack(&mut b, &sample(4, 250, 250, 40, true), 4, 120, 3, t1);
        assert_eq!(b.probe_rtt_done_stamp, Some(t1));
        assert_eq!(b.next_rtt_delivered, 120);
        ack(
            &mut b,
            &sample(4, 250, 250, 60, true),
            4,
            130,
            3,
            at(t1, 250),
        );
        assert_eq!(
            b.mode,
            BbrMode::ProbeRtt,
            "dwell is over but the round is not"
        );
        ack(
            &mut b,
            &sample(4, 250, 250, 120, true),
            4,
            140,
            3,
            at(t1, 260),
        );
        assert_eq!(b.mode, BbrMode::ProbeBw);
    }
}

mod probe_rtt_cwnd_cap {
    use super::*;

    #[test]
    fn recovery_entry_inside_probe_rtt_keeps_floor_cwnd() {
        // C udx_bbr.c:247-249: the ProbeRtt cap sits after `done:`, past every early exit.
        let t0 = Instant::now();
        let mut b = in_probe_rtt(t0, 100);
        let rs = sample(4, 250, 250, 50, true);
        let (cwnd, _, _) = b.on_ack(&rs, 4, 40, MSS, 54, 0, 4, CaState::Recovery, at(t0, 10));
        assert_eq!(
            cwnd, BBR_CWND_MIN_TARGET,
            "packet conservation alone gives 4 + 4"
        );
    }

    #[test]
    fn ack_without_delivery_inside_probe_rtt_keeps_floor_cwnd() {
        let t0 = Instant::now();
        let mut b = in_probe_rtt(t0, 100);
        let cwnd = ack(&mut b, &RateSample::default(), 40, 100, 4, at(t0, 10));
        assert_eq!(
            cwnd, BBR_CWND_MIN_TARGET,
            "the rs.delivered == 0 exit alone keeps 40"
        );
    }
}

mod probe_rtt_exit_on_transmit {
    use super::*;

    #[test]
    fn app_limited_transmit_exits_once_dwell_is_over() {
        // C udx_bbr.c:134-148: app-limited transmit in ProbeRtt runs bbr_check_probe_rtt_done.
        let t0 = Instant::now();
        let mut b = in_probe_rtt(t0, 100);
        b.on_transmit_start(false, MSS, at(t0, 300));
        assert_eq!(b.mode, BbrMode::ProbeRtt, "not app-limited: no check");
        b.on_transmit_start(true, MSS, at(t0, 100));
        assert_eq!(b.mode, BbrMode::ProbeRtt, "dwell 100 ms is not over 200 ms");
        b.on_transmit_start(true, MSS, at(t0, 250));
        assert_eq!(b.mode, BbrMode::ProbeBw);
        assert_eq!(b.cycle_index, 4);
    }

    #[test]
    fn cwnd_comes_back_on_the_next_ack() {
        let t0 = Instant::now();
        let mut b = in_probe_rtt(t0, 100);
        b.on_transmit_start(true, MSS, at(t0, 250));
        let cwnd = ack(
            &mut b,
            &sample(4, 250, 250, 50, true),
            4,
            54,
            0,
            at(t0, 260),
        );
        assert_eq!(cwnd, 104, "max(4, 100) + 4 acked");
    }

    #[test]
    fn loss_before_that_ack_saves_the_restored_cwnd() {
        // C restores at once (udx_bbr.c:442-450), so bbr_save_cwnd on loss sees prior_cwnd.
        let t0 = Instant::now();
        let mut b = in_probe_rtt(t0, 100);
        b.on_transmit_start(true, MSS, at(t0, 250));
        b.save_cwnd(4);
        assert_eq!(b.mode, BbrMode::ProbeBw);
        assert_eq!(b.prior_cwnd, 100);
    }
}

// ─── Model basics ───

/// Construct a RateSample with the fields accessible from this crate.
fn make_rs(
    delivered: u32,
    interval_ms: u32,
    prior_delivered: u32,
    app_limited: bool,
) -> RateSample {
    let mut rs = RateSample::default();
    rs.delivered = delivered;
    rs.interval_ms = interval_ms;
    rs.rtt_ms = interval_ms;
    rs.prior_delivered = prior_delivered;
    rs.is_app_limited = app_limited;
    rs
}

// ── 1. Startup gains after new() ─────────────────────────────────────────

#[test]
fn test_new_startup_gains() {
    let state = BbrState::new(100);
    assert_eq!(state.mode, BbrMode::Startup);
    assert_eq!(state.pacing_gain, BBR_HIGH_GAIN);
    assert_eq!(state.cwnd_gain, BBR_HIGH_GAIN);
}

// ── 2. Full-BW detection ─────────────────────────────────────────────────

#[test]
fn test_full_bw_detection() {
    let mut state = BbrState::new(100);
    let now = Instant::now();
    let mss = 1180_u32;
    let cwnd = 10_u32;

    // Round 0: establishes full_bw baseline (bw >= 0.0 * 1.25 == 0.0).
    let rs = make_rs(10, 10, 0, false);
    state.on_ack(&rs, 10, cwnd, mss, 10, 0, 5, CaState::Open, now);

    // Rounds 1-3: same rate, no 25 % growth → count increments each round.
    for i in 1_u32..=3 {
        let rs = make_rs(10, 10, i * 10, false);
        state.on_ack(&rs, 10, cwnd, mss, (i + 1) * 10, 0, 5, CaState::Open, now);
    }

    assert_eq!(state.full_bw_count, BBR_FULL_BW_COUNT);
}

// ── 3. Drain → ProbeBw transition ────────────────────────────────────────

#[test]
fn test_drain_to_probe_bw() {
    let mut state = BbrState::new(100);
    let now = Instant::now();
    let mss = 1180_u32;
    let cwnd = 10_u32;

    // Drive to Drain: 4 rounds of constant BW with inflight >> target so that
    // check_drain does not fire in the same tick as check_full_bw_reached.
    // After min_rtt_ms settles to 10 ms, target ≈ bw(1.0) × 10 × gain(2) = 20 pkts.
    for i in 0_u32..4 {
        let rs = make_rs(10, 10, i * 10, false);
        state.on_ack(&rs, 10, cwnd, mss, (i + 1) * 10, 0, 100, CaState::Open, now);
    }
    assert_eq!(state.mode, BbrMode::Drain);

    // Drop inflight below the target → transition to ProbeBw.
    let rs = make_rs(10, 10, 40, false);
    state.on_ack(&rs, 10, cwnd, mss, 50, 0, 1, CaState::Open, now);

    assert_eq!(state.mode, BbrMode::ProbeBw);
}

// ── 4. ProbeBw cycle advance ─────────────────────────────────────────────

#[test]
fn test_probe_bw_cycle_advance() {
    let mut state = BbrState::new(10);
    let now = Instant::now();
    let mss = 1180_u32;
    let cwnd = 10_u32;

    // Drive to ProbeBw.
    for i in 0_u32..4 {
        let rs = make_rs(10, 10, i * 10, false);
        state.on_ack(&rs, 10, cwnd, mss, (i + 1) * 10, 0, 5, CaState::Open, now);
    }
    let rs = make_rs(10, 10, 40, false);
    state.on_ack(&rs, 10, cwnd, mss, 50, 0, 1, CaState::Open, now);
    assert_eq!(state.mode, BbrMode::ProbeBw);

    let before = state.cycle_index;

    // Advance time past min_rtt_ms so the cycle phase advances.
    let later = now + Duration::from_millis(state.min_rtt_ms as u64 + 5);
    let rs = make_rs(10, 10, 50, false);
    state.on_ack(&rs, 10, cwnd, mss, 60, 0, 1, CaState::Open, later);

    let expected = (before + 1) % BBR_CYCLE_LEN;
    assert_eq!(state.cycle_index, expected);
}

// ── 5. on_rto resets ─────────────────────────────────────────────────────

#[test]
fn test_on_rto_resets() {
    // C bbr_on_rto (udx_bbr.c:609-615) zeroes only full_bw; the full-pipe flag survives.
    let mut state = BbrState::new(100);
    state.full_bw = 5.0;
    state.full_bw_count = BBR_FULL_BW_COUNT;
    state.full_bw_reached = true;
    state.round_start = false;
    state.prev_ca_state = CaState::Open;
    state.on_rto();
    assert_eq!(state.full_bw, 0.0);
    assert!(state.round_start);
    assert_eq!(state.prev_ca_state, CaState::Loss);
    assert!(state.full_bw_reached);
    assert_eq!(state.full_bw_count, BBR_FULL_BW_COUNT);
}

// ── 6. save_cwnd behaviour ───────────────────────────────────────────────

#[test]
fn test_save_cwnd() {
    let mut state = BbrState::new(100);
    state.save_cwnd(42);
    assert_eq!(state.prior_cwnd, 42);

    let mut probe = BbrState::new(100);
    probe.mode = BbrMode::ProbeRtt;
    probe.save_cwnd(42);
    assert_eq!(probe.prior_cwnd, 42);
}

// ── 7. set_pacing_rate applies 0.99 margin ───────────────────────────────

#[test]
fn test_set_pacing_rate_margin() {
    let state = BbrState::new(100);
    // 1.0 packets/ms × 1180 bytes/pkt × 1.0 gain × 0.99 = 1168.2 → 1168
    let pacing = state.set_pacing_rate(1.0, 1180, 1.0);
    assert_eq!(pacing, 1168);
}

// ── 8. set_cwnd never goes below BBR_CWND_MIN_TARGET ─────────────────────

#[test]
fn test_set_cwnd_minimum() {
    let mut state = BbrState::new(100);
    let rs = make_rs(1, 10, 0, false);
    // cwnd=0, acked=0 → must return at least BBR_CWND_MIN_TARGET.
    let new_cwnd = state.set_cwnd(
        &rs,
        0,
        0,
        0.0,
        BBR_CWND_GAIN,
        100,
        0,
        1180,
        CaState::Open,
        0,
    );
    assert!(new_cwnd >= BBR_CWND_MIN_TARGET);
}

// ── 9. min_rtt_ms() never returns 0 ─────────────────────────────────────

#[test]
fn test_min_rtt_ms_nonzero() {
    let state = BbrState::new(0);
    assert!(state.min_rtt_ms() >= 1);
}

mod bw_filter {
    use super::*;

    #[test]
    fn probe_rtt_dip_never_becomes_the_max_while_older_samples_are_in_window() {
        // C win_filter_f64.c:38-63: win/4 and win/2 refresh sub-window entries; expiry is `dt > win`.
        let mut f = WindowedMaxFilter::new();
        let path = [
            (10.0, 0),
            (9.0, 4),
            (9.0, 7),
            (2.0, 10),
            (2.0, 11),
            (9.0, 12),
        ];
        let maxes: Vec<f64> = path
            .iter()
            .map(|&(val, round)| {
                f.update(val, round, BBR_BW_FILTER_CYCLES);
                f.get()
            })
            .collect();
        assert_eq!(maxes, [10.0, 10.0, 10.0, 10.0, 9.0, 9.0]);
    }

    #[test]
    fn window_expires_strictly_after_window_rounds() {
        let mut f = WindowedMaxFilter::new();
        f.update(5.0, 0, BBR_BW_FILTER_CYCLES);
        f.update(3.0, 10, BBR_BW_FILTER_CYCLES);
        assert_eq!(f.get(), 5.0, "age 10 is not over a window of 10");
        f.update(3.0, 11, BBR_BW_FILTER_CYCLES);
        assert_eq!(f.get(), 3.0);
    }

    #[test]
    fn app_limited_sample_equal_to_the_max_refreshes_it() {
        // C udx_bbr.c:341: `bw >= bbr_max_bw(stream)`.
        let mut b = BbrState::new(250);
        for k in 1..=12u32 {
            let (delivered, app_limited) = match k {
                1 => (10, false),
                8 => (10, true),
                12 => (5, false),
                _ => (1, true),
            };
            b.update_bw(
                &sample(delivered, 10, 10, 10 * (k - 1), app_limited),
                10 * k,
            );
        }
        assert_eq!(
            b.bw, 1.0,
            "the round-8 sample keeps 1.0 in the window at round 12"
        );
    }
}

mod update_bw_round {
    use super::*;

    /// Filter state at round 5, boundary at 100 delivered.
    fn at_round_5() -> BbrState {
        let mut b = BbrState::new(250);
        b.round_count = 5;
        b.next_rtt_delivered = 100;
        b
    }

    #[test]
    fn discarded_sample_past_the_boundary_does_not_move_the_round() {
        // C udx_bbr.c:322-324: early return before round detection.
        let mut b = at_round_5();
        b.round_start = true;
        b.update_bw(&sample(0, 10, 10, 150, false), 200);
        assert_eq!(b.round_count, 5);
        assert!(!b.round_start);
        assert_eq!(b.next_rtt_delivered, 100);
    }

    #[test]
    fn discarded_sample_keeps_packet_conservation() {
        let mut b = at_round_5();
        b.packet_conservation = true;
        b.update_bw(&sample(0, 10, 10, 150, false), 200);
        assert!(b.packet_conservation);
    }

    #[test]
    fn zero_interval_sample_does_not_move_the_round() {
        let mut b = at_round_5();
        b.packet_conservation = true;
        b.update_bw(&sample(5, 0, 10, 150, false), 200);
        assert_eq!(b.round_count, 5);
        assert!(!b.round_start);
        assert_eq!(b.next_rtt_delivered, 100);
        assert!(b.packet_conservation);
    }

    #[test]
    fn valid_sample_past_the_boundary_opens_round_and_clears_conservation() {
        // C udx_bbr.c:331-335.
        let mut b = at_round_5();
        b.packet_conservation = true;
        b.update_bw(&sample(5, 10, 10, 100, false), 200);
        assert_eq!(b.round_count, 6);
        assert!(b.round_start);
        assert!(!b.packet_conservation);
        assert_eq!(b.next_rtt_delivered, 200);
    }

    #[test]
    fn valid_sample_before_the_boundary_leaves_round_and_conservation() {
        let mut b = at_round_5();
        b.packet_conservation = true;
        b.update_bw(&sample(5, 10, 10, 99, false), 200);
        assert_eq!(b.round_count, 5);
        assert!(!b.round_start);
        assert!(b.packet_conservation);
        assert_eq!(b.next_rtt_delivered, 100);
    }

    #[test]
    fn conserved_window_does_not_grow_until_a_round_opens() {
        let t0 = Instant::now();
        let mut b = full_pipe(t0);
        b.prev_ca_state = CaState::Recovery;
        b.packet_conservation = true;
        b.next_rtt_delivered = 100;
        let rs = sample(10, 10, 250, 50, false);
        let grow = |b: &mut BbrState| {
            b.set_cwnd(&rs, 10, 50, 1.0, 2.0, 250, 20, MSS, CaState::Recovery, 60)
        };
        assert_eq!(grow(&mut b), 50, "conserved: no growth to target");
        b.update_bw(&sample(10, 10, 250, 100, false), 110);
        assert!(!b.packet_conservation);
        assert_eq!(grow(&mut b), 60, "round opened: window grows by acked");
    }

    /// C udx_bbr.c:330 compares with `seq_diff(...) >= 0`: the counters wrap at 2^32, and a plain
    /// `>=` would freeze the round counter for a whole wrap once `delivered` passes u32::MAX.
    #[test]
    fn the_round_boundary_survives_the_counter_wrap() {
        let mut b = at_round_5();
        b.next_rtt_delivered = u32::MAX - 10;

        // Before the boundary, on the far side of the wrap: the round must not open.
        b.update_bw(&sample(5, 10, 10, u32::MAX - 20, false), 7);
        assert_eq!(b.round_count, 5);
        assert!(!b.round_start);
        assert_eq!(b.next_rtt_delivered, u32::MAX - 10);

        // Past the boundary, already wrapped: the round opens.
        b.update_bw(&sample(5, 10, 10, 5, false), 7);
        assert_eq!(b.round_count, 6);
        assert!(b.round_start);
        assert_eq!(b.next_rtt_delivered, 7);
    }
}
