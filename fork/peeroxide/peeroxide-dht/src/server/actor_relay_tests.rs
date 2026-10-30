//! Tests for the punching deadline of a slot.

use super::*;
use std::time::Duration;

const S5: Duration = Duration::from_secs(5);

#[test]
fn deadline_without_held_relay_is_the_window_from_the_round() {
    let now = Instant::now();
    let d = punching_deadline(now, None);
    assert_eq!(d, now + PUNCHING_TIMEOUT);
    assert!(d > now + INITIAL_TIMEOUT);
}

#[test]
fn held_relay_later_than_the_window_is_not_shortened() {
    let now = Instant::now();
    let held = now + PUNCHING_TIMEOUT + S5;
    assert_eq!(punching_deadline(now, Some(held)), held);
}

#[test]
fn held_relay_earlier_than_the_window_gives_way_to_it() {
    let now = Instant::now();
    assert_eq!(
        punching_deadline(now, Some(now + S5)),
        now + PUNCHING_TIMEOUT
    );
}
