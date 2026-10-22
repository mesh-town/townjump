//! `UnknownIdLog` without a socket.

use super::demux_log::UnknownIdLog;
use std::time::{Duration, Instant};

#[test]
fn first_note_true() {
    let mut log = UnknownIdLog::default();
    assert!(log.note(7, Instant::now()));
}

#[test]
fn second_within_1s_false() {
    let mut log = UnknownIdLog::default();
    let t = Instant::now();
    assert!(log.note(7, t));
    assert!(!log.note(7, t + Duration::from_millis(999)));
    assert!(log.note(8, t), "another id is independent");
}

#[test]
fn after_1s_true() {
    let mut log = UnknownIdLog::default();
    let t = Instant::now();
    assert!(log.note(7, t));
    assert!(log.note(7, t + Duration::from_secs(1)));
}

#[test]
fn eviction_over_1024() {
    let mut log = UnknownIdLog::default();
    let t = Instant::now();
    for id in 0..1025u32 {
        assert!(log.note(id, t), "id {id}");
        assert!(log.len() <= 1024);
    }
}
