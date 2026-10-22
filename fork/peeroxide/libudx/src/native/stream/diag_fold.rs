//! `ZS` fold (instrument Z): sender steps, RACK walks and writer wakeups accumulate and are printed at most once per 10 ms.
use super::now_us;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Why the step gate closed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::native::stream) enum Why {
    Pace,
    Win,
    Queue,
}

/// Outcome of one sender step for the fold.
pub(in crate::native::stream) struct Step {
    pub(in crate::native::stream) sent_new: u32,
    pub(in crate::native::stream) sent_rtx: u32,
    pub(in crate::native::stream) why: Why,
    pub(in crate::native::stream) cwnd: u32,
    pub(in crate::native::stream) inflight: u32,
    pub(in crate::native::stream) lost: u32,
    pub(in crate::native::stream) sacked: u32,
    pub(in crate::native::stream) ca_state: u8,
    /// `congestion.pacing_bytes_per_ms`.
    pub(in crate::native::stream) rate: u32,
    /// The step's `batch.pacing_delay`, us; 0 — there was none.
    pub(in crate::native::stream) pace_delay_us: u64,
}

/// Outcome of handling one incoming ACK for the fold (ACK side).
pub(in crate::native::stream) struct AckProbe {
    /// The ACK armed the step timer (`kick_pacing`).
    pub(in crate::native::stream) kick: bool,
    /// After the ACK the send window is open (`can_send`).
    pub(in crate::native::stream) open: bool,
    /// The writer is parked (`writer_waker.is_some()`).
    pub(in crate::native::stream) parked: bool,
    /// `write_room() == 0`.
    pub(in crate::native::stream) room0: bool,
    pub(in crate::native::stream) unsacked: usize,
    /// Packets in `send_queue`.
    pub(in crate::native::stream) q_len: usize,
    pub(in crate::native::stream) cwnd: u32,
    pub(in crate::native::stream) lost: u32,
}

struct Fold {
    start: Option<Instant>,
    steps: u32,
    sent_new: u32,
    sent_rtx: u32,
    empty: u32,
    pace: u32,
    win: u32,
    queue: u32,
    pace_delay_max_us: u64,
    /// Values at the last step of the window.
    cwnd: u32,
    inflight: u32,
    lost: u32,
    sacked: u32,
    ca_state: u8,
    rate: u32,
    rack_calls: u32,
    rack_sum: u64,
    rack_max: u32,
    w_wake: u32,
    w_blocked: u32,
    w_room_min: Option<usize>,
    a_n: u32,
    a_kick: u32,
    a_open_nokick: u32,
    a_open_room0: u32,
    a_parked_room0: u32,
    unsacked_max: usize,
    q_len_max: usize,
    cwnd_min: Option<u32>,
    lost_max: u32,
    ack_proc_us_max: u64,
    far_armed: u32,
    step_kick: u32,
    step_self: u32,
    kick_to_step_n: u32,
    kick_to_step_us_sum: u64,
    kick_to_step_us_max: u64,
    step_inline: u32,
}

impl Fold {
    const ZERO: Fold = Fold {
        start: None,
        steps: 0,
        sent_new: 0,
        sent_rtx: 0,
        empty: 0,
        pace: 0,
        win: 0,
        queue: 0,
        pace_delay_max_us: 0,
        cwnd: 0,
        inflight: 0,
        lost: 0,
        sacked: 0,
        ca_state: 0,
        rate: 0,
        rack_calls: 0,
        rack_sum: 0,
        rack_max: 0,
        w_wake: 0,
        w_blocked: 0,
        w_room_min: None,
        a_n: 0,
        a_kick: 0,
        a_open_nokick: 0,
        a_open_room0: 0,
        a_parked_room0: 0,
        unsacked_max: 0,
        q_len_max: 0,
        cwnd_min: None,
        lost_max: 0,
        ack_proc_us_max: 0,
        far_armed: 0,
        step_kick: 0,
        step_self: 0,
        kick_to_step_n: 0,
        kick_to_step_us_sum: 0,
        kick_to_step_us_max: 0,
        step_inline: 0,
    };

    fn print(&self, dt: Duration) {
        eprintln!("{}", self.zs_line(dt));
    }

    fn note_ack(&mut self, p: &AckProbe) {
        self.a_n += 1;
        self.a_kick += u32::from(p.kick);
        self.a_open_nokick += u32::from(p.open && !p.kick);
        self.a_open_room0 += u32::from(p.open && p.parked && p.room0);
        self.a_parked_room0 += u32::from(p.parked && p.room0);
        self.unsacked_max = self.unsacked_max.max(p.unsacked);
        self.q_len_max = self.q_len_max.max(p.q_len);
        self.cwnd_min = Some(self.cwnd_min.map_or(p.cwnd, |m| m.min(p.cwnd)));
        self.lost_max = self.lost_max.max(p.lost);
    }

    fn note_kick_to_step(&mut self, us: u64) {
        self.kick_to_step_n += 1;
        self.kick_to_step_us_sum += us;
        self.kick_to_step_us_max = self.kick_to_step_us_max.max(us);
    }

    /// Text of the `ZS` line for a window of length `dt`.
    fn zs_line(&self, dt: Duration) -> String {
        format!(
            "ZS t={} dt={} steps={} sent_new={} sent_rtx={} empty={} why_pace={} why_win={} why_queue={} cwnd={} inflight={} lost={} sacked={} ca_state={} rate={} pace_delay_max_us={} rack_calls={} rack_visited_sum={} rack_visited_max={} w_wake={} w_blocked={} w_room_min={} a_n={} a_kick={} a_open_nokick={} a_open_room0={} a_parked_room0={} unsacked_max={} q_len_max={} cwnd_min={} lost_max={} ack_proc_us_max={} far_armed={} step_kick={} step_self={} kick_to_step_n={} kick_to_step_us_sum={} kick_to_step_us_max={} step_inline={}",
            now_us(),
            dt.as_micros(),
            self.steps,
            self.sent_new,
            self.sent_rtx,
            self.empty,
            self.pace,
            self.win,
            self.queue,
            self.cwnd,
            self.inflight,
            self.lost,
            self.sacked,
            self.ca_state,
            self.rate,
            self.pace_delay_max_us,
            self.rack_calls,
            self.rack_sum,
            self.rack_max,
            self.w_wake,
            self.w_blocked,
            self.w_room_min.map_or(-1, |r| r as i64),
            self.a_n,
            self.a_kick,
            self.a_open_nokick,
            self.a_open_room0,
            self.a_parked_room0,
            self.unsacked_max,
            self.q_len_max,
            self.cwnd_min.map_or(-1, i64::from),
            self.lost_max,
            self.ack_proc_us_max,
            self.far_armed,
            self.step_kick,
            self.step_self,
            self.kick_to_step_n,
            self.kick_to_step_us_sum,
            self.kick_to_step_us_max,
            self.step_inline
        )
    }
}

static FOLD: Mutex<Fold> = Mutex::new(Fold::ZERO);

fn fold() -> MutexGuard<'static, Fold> {
    FOLD.lock().unwrap_or_else(|e| e.into_inner())
}

/// A sender step. The `ZS` line covers the window from the previous reset to this call, the CURRENT step already goes into the new fold
/// (`dt=300 steps=0` — there were no steps); printing at most once per 10 ms.
pub(in crate::native::stream) fn step(s: &Step) {
    let now = Instant::now();
    let mut f = fold();
    let start = *f.start.get_or_insert(now);
    let dt = now.duration_since(start);
    if dt >= Duration::from_millis(10) {
        f.print(dt);
        *f = Fold {
            start: Some(now),
            ..Fold::ZERO
        };
    }
    f.steps += 1;
    f.sent_new += s.sent_new;
    f.sent_rtx += s.sent_rtx;
    f.empty += u32::from(s.sent_new + s.sent_rtx == 0);
    match s.why {
        Why::Pace => f.pace += 1,
        Why::Win => f.win += 1,
        Why::Queue => f.queue += 1,
    }
    f.pace_delay_max_us = f.pace_delay_max_us.max(s.pace_delay_us);
    (f.cwnd, f.inflight, f.lost, f.sacked, f.ca_state, f.rate) =
        (s.cwnd, s.inflight, s.lost, s.sacked, s.ca_state, s.rate);
}

/// One RACK walk: how many entries `take_while` passed.
pub(in crate::native::stream) fn rack(visited: u32) {
    let mut f = fold();
    f.rack_calls += 1;
    f.rack_sum += u64::from(visited);
    f.rack_max = f.rack_max.max(visited);
}

/// Writer on ACK: woken (`woke`) or left waiting; `room` — write headroom.
pub(in crate::native::stream) fn writer(woke: bool, room: usize) {
    let mut f = fold();
    if woke {
        f.w_wake += 1;
    } else {
        f.w_blocked += 1;
    }
    f.w_room_min = Some(f.w_room_min.map_or(room, |m| m.min(room)));
}

/// Incoming ACK handled (after the writer was released): sender and writer state.
pub(in crate::native::stream) fn ack(p: &AckProbe) {
    fold().note_ack(p);
}

/// Time to handle one incoming packet in `incoming.rs`, us.
pub(in crate::native::stream) fn ack_proc(us: u64) {
    let mut f = fold();
    f.ack_proc_us_max = f.ack_proc_us_max.max(us);
}

/// `after_step` set the step timer to `FAR_FUTURE`.
pub(in crate::native::stream) fn far_armed() {
    fold().far_armed += 1;
}

/// The step was started by a timer: armed by ACK/write/RTO (`by_kick`) or by `after_step` itself per `pacing_delay`.
pub(in crate::native::stream) fn step_src(by_kick: bool) {
    let mut f = fold();
    if by_kick {
        f.step_kick += 1;
    } else {
        f.step_self += 1;
    }
}

/// Delay kick -> step by timer, us.
pub(in crate::native::stream) fn kick_to_step(us: u64) {
    fold().note_kick_to_step(us);
}

/// The step was started inline after an ACK packet (`UDX_FIX=ackstep`).
pub(in crate::native::stream) fn step_inline() {
    fold().step_inline += 1;
}

#[cfg(test)]
#[path = "diag_fold_tests.rs"]
mod tests;
