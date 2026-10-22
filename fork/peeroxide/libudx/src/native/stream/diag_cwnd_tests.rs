//! The `ZC` line: format and the "at most once per 2 ms, except on a branch change" rule.
use super::{ZcRow, should_print, zc_line};
use std::time::{Duration, Instant};

#[test]
fn line_has_all_keys_in_order() {
    let row = ZcRow {
        cwnd_before: 1196,
        cwnd_after: 4,
        branch: "conservation",
        inflight_arg: 1196,
        acked: 3,
        losses: 0,
        target: 25,
        ca_state: 2,
    };
    assert_eq!(
        zc_line(7, &row),
        "ZC t=7 cwnd_before=1196 cwnd_after=4 branch=conservation inflight_arg=1196 acked=3 losses=0 target=25 ca_state=2"
    );
}

#[test]
fn first_line_always_prints() {
    assert!(should_print(None, Instant::now(), "grow"));
}

#[test]
fn same_branch_within_2ms_is_skipped() {
    let t0 = Instant::now();
    let last = Some((t0, "grow"));
    assert!(!should_print(
        last,
        t0 + Duration::from_micros(1999),
        "grow"
    ));
    assert!(should_print(last, t0 + Duration::from_millis(2), "grow"));
}

#[test]
fn other_branch_prints_at_once() {
    let t0 = Instant::now();
    assert!(should_print(Some((t0, "grow")), t0, "enter"));
}
