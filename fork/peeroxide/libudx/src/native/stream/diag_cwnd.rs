//! The `ZC` line (Z probe): a `cwnd` change. Printed on a value change, at most once per 2 ms, except on a branch change.
//! `t` is microseconds since UNIX_EPOCH, as in `ZS`; `-1` in a numeric field means "unknown on this branch".
use super::now_us;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Minimum interval between `ZC` lines of the same branch.
const MIN_GAP: Duration = Duration::from_millis(2);

/// One `cwnd` change. Branches: `enter`, `conservation`, `restore`, `delivered0`, `full_bw`, `grow` (BBR `set_cwnd`),
/// `probe_rtt` (ProbeRtt cut), `rto_cut` (window after RTO), `mtu` (recomputation after an MTU change).
pub(in crate::native) struct ZcRow {
    pub(in crate::native) cwnd_before: u32,
    pub(in crate::native) cwnd_after: u32,
    pub(in crate::native) branch: &'static str,
    /// The `inflight` that `set_cwnd` ran with (the same as in the Z lines).
    pub(in crate::native) inflight_arg: i64,
    pub(in crate::native) acked: i64,
    pub(in crate::native) losses: i64,
    /// `target_cwnd + extra_acked`; 0 means the branch did not get that far.
    pub(in crate::native) target: i64,
    pub(in crate::native) ca_state: u8,
}

/// Text of the `ZC` line for moment `t` (us).
fn zc_line(t: u128, r: &ZcRow) -> String {
    format!(
        "ZC t={t} cwnd_before={} cwnd_after={} branch={} inflight_arg={} acked={} losses={} target={} ca_state={}",
        r.cwnd_before,
        r.cwnd_after,
        r.branch,
        r.inflight_arg,
        r.acked,
        r.losses,
        r.target,
        r.ca_state
    )
}

/// Whether to print: the first line, a branch different from the last printed one, or at least 2 ms have passed.
fn should_print(last: Option<(Instant, &'static str)>, now: Instant, branch: &str) -> bool {
    match last {
        None => true,
        Some((at, prev)) => prev != branch || now.saturating_duration_since(at) >= MIN_GAP,
    }
}

static LAST: Mutex<Option<(Instant, &'static str)>> = Mutex::new(None);

/// A `cwnd` change; the caller checked `diag::mode().z` and that the value changed.
pub(in crate::native) fn cwnd_change(r: &ZcRow) {
    let now = Instant::now();
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    if !should_print(*last, now, r.branch) {
        return;
    }
    *last = Some((now, r.branch));
    eprintln!("{}", zc_line(now_us(), r));
}

#[cfg(test)]
#[path = "diag_cwnd_tests.rs"]
mod tests;
