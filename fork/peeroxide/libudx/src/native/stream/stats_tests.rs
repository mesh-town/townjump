//! `stats()` tests: initial values, inflight after sending, reading substituted fields.
use super::*;
use crate::native::stream::test_support::{rto_resend, stream_inner, with_sent};

#[test]
fn fresh_stream_has_initial_values() {
    let inner = stream_inner();
    let s = inner.stats();
    assert_eq!((s.cwnd, s.ssthresh, s.inflight), (10, 0xffff, 0));
    assert_eq!((s.pacing_bytes_per_ms, s.srtt_ms), (0, 0));
    assert_eq!((s.delivered, s.lost), (0, 0));
    assert_eq!(s.rto_ms, inner.rto);
    assert_eq!((s.mode, s.full_bw_reached), (0, 0));
    assert_eq!((s.app_limited, s.bw_bytes_per_ms), (1, 0));
}

#[test]
fn inflight_counts_sent_packets() {
    let mut inner = stream_inner();
    with_sent(&mut inner, 100, 5);
    // 5 sent without SACK and without losses — all in flight
    assert_eq!(inner.stats().inflight, 5);
}

#[test]
fn snapshot_reads_current_fields() {
    let mut inner = stream_inner();
    inner.srtt = 231;
    inner.rto = 1500;
    inner.congestion.pacing_bytes_per_ms = 9000;
    inner.congestion.cwnd = 700;
    inner.congestion.app_limited = 0;
    let s = inner.stats();
    assert_eq!(s.app_limited, 0);
    assert_eq!((s.srtt_ms, s.rto_ms), (231, 1500));
    assert_eq!((s.pacing_bytes_per_ms, s.cwnd), (9000, 700));
}

#[test]
fn public_handle_reports_initial_cwnd() {
    assert_eq!(UdxStream::new(7).stats().cwnd, 10);
}

#[test]
fn mode_codes_match_c_bbr_states() {
    use crate::native::congestion::BbrMode;
    let codes = [
        BbrMode::Startup,
        BbrMode::Drain,
        BbrMode::ProbeBw,
        BbrMode::ProbeRtt,
    ]
    .map(|m| m as u32);
    assert_eq!(codes, [0, 1, 2, 3]);
}

#[test]
fn bw_converts_packets_per_ms_to_bytes_per_ms() {
    assert_eq!(bw_bytes_per_ms(2.5, 1152), 2880);
    assert_eq!(bw_bytes_per_ms(f64::NAN, 1152), 0);
    assert_eq!(bw_bytes_per_ms(-1.0, 1152), 0);
    assert_eq!(bw_bytes_per_ms(1e12, 1152), u32::MAX);
}

#[test]
fn fresh_stream_has_zero_retransmit_counters() {
    let s = stream_inner().stats();
    assert_eq!((s.rto_fires, s.resent), (0, 0));
}

#[test]
fn rto_fire_counts_once_per_fire() {
    let mut inner = stream_inner();
    inner.on_rto_fire(); // outgoing is empty — not counted
    assert_eq!(inner.stats().rto_fires, 0);
    with_sent(&mut inner, 100, 3);
    inner.on_rto_fire();
    assert_eq!(inner.stats().rto_fires, 1);
    inner.on_rto_fire();
    assert_eq!(inner.stats().rto_fires, 2);
}

#[test]
fn rto_resend_adds_packet_count() {
    let mut inner = stream_inner();
    with_sent(&mut inner, 100, 5);
    assert_eq!(rto_resend(&mut inner).len(), 5);
    assert_eq!(inner.stats().resent, 5);
    rto_resend(&mut inner);
    assert_eq!(inner.stats().resent, 10);
}

#[test]
fn min_rtt_reads_bbr_minimum_in_ms() {
    let mut inner = stream_inner();
    assert_eq!(inner.stats().min_rtt, 1);
    let t0 = Instant::now();
    let info = inner.congestion.on_packet_sent(1, 0, t0);
    inner.congestion.on_ack(
        &[(1, info)],
        0,
        0,
        t0 + std::time::Duration::from_millis(40),
    );
    assert_eq!(inner.stats().min_rtt, 40);
}
