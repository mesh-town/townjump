//! Instrument `ZP`, `ap_*` fields: buckets, quantiles, format, reset after printing.
use super::{View, Zp};

fn view() -> View {
    View {
        remote_acked: 0,
        seq: 0,
        map_len: 0,
        map_min: 0,
        sacked: 0,
        lost: 0,
        inflight: 0,
        cwnd_cmp: 0,
        cwnd_lim: 0,
        rack_rtt_ms: 0,
        reo_wnd_ms: 0,
        min_rtt_ms: None,
    }
}

#[test]
fn quantiles_on_synthetic_set() {
    let mut zp = Zp::default();
    for _ in 0..100 {
        zp.note_ack_proc(10);
    }
    zp.note_ack_proc(5000);
    // 10 us: floor(log2(11)) = 3 -> bound 15; 99 % of 101 is already reached in this bucket.
    let line = zp.line(0, &view());
    assert!(
        line.contains("ap_n=101 ap_sum_us=6000 ap_p50_us=15 ap_p99_us=15 ap_max_us=5000"),
        "{line}"
    );
}

#[test]
fn p99_reaches_tail_bucket() {
    let mut zp = Zp::default();
    for _ in 0..98 {
        zp.note_ack_proc(10);
    }
    for _ in 0..2 {
        zp.note_ack_proc(5000);
    }
    // 5000 us: floor(log2(5001)) = 12 -> bound 8191; 98 % < 99 %, p99 goes into the tail.
    let line = zp.line(0, &view());
    assert!(
        line.contains("ap_p50_us=15 ap_p99_us=8191 ap_max_us=5000"),
        "{line}"
    );
}

#[test]
fn window_resets_and_clamps() {
    let mut zp = Zp::default();
    zp.note_ack_proc(0);
    zp.note_ack_proc(u64::from(u32::MAX));
    let line = zp.line(0, &view());
    assert!(
        line.contains("ap_n=2 ") && line.contains("ap_p99_us=33554431"),
        "{line}"
    );
    let line = zp.line(1, &view());
    assert!(
        line.contains("ap_n=0 ap_sum_us=0 ap_p50_us=0 ap_p99_us=0 ap_max_us=0"),
        "{line}"
    );
}
