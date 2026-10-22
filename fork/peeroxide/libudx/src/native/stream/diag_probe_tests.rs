//! Instrument `ZP`: line format, clock once a second, mark counters on a synthetic loss.
use super::{View, Zp};
use std::time::{Duration, Instant};

fn view() -> View {
    View {
        remote_acked: 10,
        seq: 20,
        map_len: 9,
        map_min: 11,
        sacked: 3,
        lost: 2,
        inflight: 4,
        cwnd_cmp: 4,
        cwnd_lim: 8,
        rack_rtt_ms: 50,
        reo_wnd_ms: 12,
        min_rtt_ms: None,
    }
}

#[test]
fn line_has_all_fields() {
    let mut zp = Zp {
        marks_ack: 5,
        marks_timer: 6,
        marks_enter: 7,
        marks_rto: 8,
        ..Zp::default()
    };
    for us in 1..=100 {
        zp.note_rx(us, (us % 7) as usize);
    }
    zp.sack_lookups = 40;
    zp.rack_visits = 90;
    zp.proc_n = 77;
    super::count_recv();
    let line = zp.line(1500, &view());
    assert!(line.starts_with("ZP t=1500 us="), "{line}");
    let kv: std::collections::HashMap<_, _> = line
        .split(' ')
        .skip(1)
        .map(|p| p.split_once('=').unwrap())
        .collect();
    for (k, want) in [
        ("remote_acked", "10"),
        ("seq", "20"),
        ("map_len", "9"),
        ("map_min", "11"),
        ("sacked", "3"),
        ("lost", "2"),
        ("inflight", "4"),
        ("cwnd_cmp", "4"),
        ("cwnd_lim", "8"),
        ("rack_rtt", "50"),
        ("reo_wnd", "12"),
        ("min_rtt", "-1"),
        ("marks_ack", "5"),
        ("marks_timer", "6"),
        ("marks_enter", "7"),
        ("marks_rto", "8"),
        ("dq_n", "100"),
        ("dq_p50_us", "51"),
        ("dq_p90_us", "91"),
        ("dq_max_us", "100"),
        ("q_last", "2"),
        ("q_max", "6"),
        ("sack_lookups", "40"),
        ("rack_visits", "90"),
        ("proc_n", "77"),
    ] {
        assert_eq!(kv.get(k), Some(&want), "{k}: {line}");
    }
    let recv_n: u64 = kv["recv_n"].parse().unwrap();
    assert!(recv_n >= 1, "{line}");
}

#[test]
fn due_once_per_second() {
    let mut zp = Zp::default();
    let t0 = Instant::now();
    assert_eq!(zp.due(t0), None);
    assert_eq!(zp.due(t0 + Duration::from_millis(999)), None);
    assert_eq!(zp.due(t0 + Duration::from_millis(1000)), Some(1000));
    assert_eq!(zp.due(t0 + Duration::from_millis(1500)), None);
    assert_eq!(zp.due(t0 + Duration::from_millis(2100)), Some(2100));
}

#[test]
fn window_resets_after_line() {
    let mut zp = Zp::default();
    zp.note_rx(500, 3);
    zp.sack_lookups = 1;
    let _ = zp.line(0, &view());
    let line = zp.line(1, &view());
    let want =
        "dq_n=0 dq_p50_us=0 dq_p90_us=0 dq_max_us=0 q_last=3 q_max=3 sack_lookups=0 rack_visits=0";
    assert!(line.contains(want), "{line}");
}

#[test]
fn ring_wraps_without_growth() {
    let mut zp = Zp::default();
    for us in 0..20_000u64 {
        zp.note_rx(us, 0);
    }
    assert_eq!(zp.delays.len(), super::RING);
    assert!(zp.line(0, &view()).contains("dq_n=20000 "));
}
