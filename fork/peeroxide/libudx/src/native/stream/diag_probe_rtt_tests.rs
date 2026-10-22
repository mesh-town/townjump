//! The `ZP` probe, `rtt_*`/`cum_n` fields: maxima, last sample, reset, line format.
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
fn samples_track_last_max_and_dq_of_sampling_acks_only() {
    let mut zp = Zp::default();
    zp.note_rx(900, 0); // ACK without a sample: its dq does not enter the maximum
    zp.rtt.note_cum();
    zp.note_rx(40, 0);
    zp.rtt.note_cum();
    zp.rtt.note_sample(220);
    zp.note_rx(70, 0);
    zp.rtt.note_cum();
    zp.rtt.note_sample(150);
    let line = zp.line(0, &view());
    assert!(
        line.ends_with("rtt_n=2 rtt_last=150 rtt_max=220 rtt_dq_max_us=70 cum_n=3"),
        "{line}"
    );
}

#[test]
fn window_resets_after_print() {
    let mut zp = Zp::default();
    zp.rtt.note_sample(5);
    zp.rtt.note_cum();
    let _ = zp.line(0, &view());
    let line = zp.line(1, &view());
    assert!(
        line.ends_with("rtt_n=0 rtt_last=0 rtt_max=0 rtt_dq_max_us=0 cum_n=0"),
        "{line}"
    );
}
