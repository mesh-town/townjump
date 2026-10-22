//! Diagnostics and stream switches via env `UDX_DIAG`: comma-separated tokens `z`, `f1`, `nof1`.
//! Empty or unset — the Z probe is off, F1 is on (default, as in C). `nof1` turns F1 off
//! (emergency fallback to leaving recovery by the SACK number); `f1` is allowed and changes nothing;
//! tokens are read left to right, the last one wins. Z lines go to stderr with the first word
//! `ZX`/`ZE`/`ZS`/`ZC`/`ZP`, then space-separated `k=v`; `t` is microseconds since UNIX_EPOCH (aligns with pcap).
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

#[path = "diag_fold.rs"]
mod fold;
pub(super) use fold::{
    AckProbe, Step, Why, ack, ack_proc, far_armed, kick_to_step, rack, step, step_inline, step_src,
    writer,
};

#[path = "diag_probe.rs"]
mod probe;
pub(in crate::native) use probe::count_recv;
pub(super) use probe::{View as ZpView, Zp};

#[path = "diag_cwnd.rs"]
mod cwnd;
pub(in crate::native) use cwnd::{ZcRow, cwnd_change};

/// What is enabled in `UDX_DIAG`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::native) struct Mode {
    /// Z lines (`ZX`, `ZE`, `ZS`, `ZC`).
    pub(in crate::native) z: bool,
    /// F1: leave recovery by the header's cumulative ACK, not by the SACK number.
    /// On by default, turned off by the `nof1` token.
    pub(in crate::native) f1: bool,
}

/// Parses the `UDX_DIAG` value; unknown tokens are ignored.
pub(super) fn parse(value: &str) -> Mode {
    let mut mode = Mode { z: false, f1: true };
    for token in value.split(',').map(str::trim) {
        match token {
            "z" => mode.z = true,
            "f1" => mode.f1 = true,
            "nof1" => mode.f1 = false,
            _ => {}
        }
    }
    mode
}

/// Mode from the environment; read once.
pub(in crate::native) fn mode() -> Mode {
    static MODE: OnceLock<Mode> = OnceLock::new();
    *MODE.get_or_init(|| parse(&std::env::var("UDX_DIAG").unwrap_or_default()))
}

pub(super) fn now_us() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_micros())
}

/// Exit from recovery/loss (`ZX`). `state_before` is the `CaState` number before (2 or 3).
pub(super) struct Exit {
    pub(super) state_before: u8,
    pub(super) cum: u32,
    pub(super) max_acked: u32,
    pub(super) high_seq: u32,
    pub(super) cum_gt_high: bool,
    pub(super) cwnd_before: u32,
    pub(super) cwnd_after: u32,
    pub(super) inflight: u32,
    pub(super) lost: u32,
    pub(super) sacked: u32,
}

pub(super) fn exit(e: &Exit) {
    eprintln!(
        "ZX t={} state_before={} cum={} max_acked={} high_seq={} cum_gt_high={} cwnd_before={} cwnd_after={} inflight={} lost={} sacked={}",
        now_us(),
        e.state_before,
        e.cum,
        e.max_acked,
        e.high_seq,
        u8::from(e.cum_gt_high),
        e.cwnd_before,
        e.cwnd_after,
        e.inflight,
        e.lost,
        e.sacked
    );
}

/// Entry into recovery (`ZE`): after `high_seq` is set.
pub(super) fn enter(high_seq: u32, cwnd: u32, inflight: u32, newly_lost: u32, cum: u32) {
    eprintln!(
        "ZE t={} high_seq={high_seq} cwnd={cwnd} inflight={inflight} newly_lost={newly_lost} cum={cum}",
        now_us()
    );
}
