// JS: libudx/src/udx.c
//! MTU tests: probe, rollback, advancement and search completion.

use super::test_support::rto_resend;
use super::*;
use std::time::Duration;

fn make_stream_inner() -> StreamInner {
    let (read_tx, _read_rx) = mpsc::unbounded_channel();
    StreamInner::new(read_tx, None)
}

// T3: MTU state starts at BASE
#[test]
fn mtu_initial_state_is_base() {
    let inner = make_stream_inner();
    assert_eq!(inner.mtu_state, MtuState::Base);
    assert_eq!(inner.mtu, MTU_BASE);
    assert!(!inner.mtu_probe_wanted);
    assert_eq!(inner.mtu_probe_count, 0);
    assert_eq!(inner.mtu_probe_size, MTU_BASE + MTU_STEP);
    assert_eq!(inner.mtu_max, MTU_MAX);
}

// T5: MTU probe failure falls back gracefully — retransmit strips padding and increments failure count
#[test]
fn mtu_probe_failure_fallback() {
    let mut inner = make_stream_inner();
    inner.mtu_state = MtuState::Search;
    inner.mtu = MTU_BASE;

    let payload = vec![0xABu8; inner.max_payload()];
    let packet = build_data_packet(0, 0, 0, &payload);
    inner.outgoing.insert(
        Seq(0),
        SentPacket {
            packet,
            time_sent: Instant::now() - Duration::from_millis(1),
            retransmit_count: 0,
            sacked: false,
            is_mtu_probe: true,
            lost: false,
            rate_info: None,
            remote_addr: None,
        },
    );

    let retransmits = rto_resend(&mut inner);
    assert_eq!(retransmits.len(), 1);
    assert_eq!(inner.mtu_probe_count, 1);
    assert!(
        !inner.outgoing[&Seq(0)].is_mtu_probe,
        "probe flag should be cleared after retransmit"
    );
    assert_eq!(
        inner.outgoing[&Seq(0)].packet[3],
        0,
        "data_offset should be reset"
    );
    assert_eq!(
        inner.mtu_state,
        MtuState::Search,
        "still searching after 1 failure"
    );
    assert!(
        inner.mtu_probe_wanted,
        "should want another probe after non-terminal failure"
    );
}

// T6: Probe uses data_offset padding — probeify inflates packet to probe size
#[test]
fn mtu_probe_uses_data_offset_padding() {
    let mut inner = make_stream_inner();
    inner.mtu_state = MtuState::Search;
    inner.mtu_probe_size = MTU_BASE + MTU_STEP;

    inner.remote_addr = Some("127.0.0.1:9".parse().unwrap());

    let payload = vec![0xCDu8; inner.max_payload()];
    let mut packet = build_data_packet(0, 0, 0, &payload);
    let ip_udp = inner.ip_overhead() - HEADER_SIZE;
    assert_eq!(
        packet.len() + ip_udp,
        MTU_BASE,
        "IP size of the original packet"
    );

    let ok = inner.mtu_probeify_packet(&mut packet);
    assert!(ok, "probeify should succeed for nearly-full packet");
    assert_eq!(
        packet.len() + ip_udp,
        MTU_BASE + MTU_STEP,
        "IP size of the padded packet equals the probe size"
    );
    assert_eq!(
        packet[3] as usize, MTU_STEP,
        "data_offset should equal padding size"
    );

    let payload_start = HEADER_SIZE + MTU_STEP;
    assert_eq!(
        &packet[payload_start..],
        &payload[..],
        "original payload should be intact after padding"
    );
}

// T7: Probe ACK advances MTU and scales congestion window
#[test]
fn mtu_probe_ack_advances_mtu() {
    let mut inner = make_stream_inner();
    inner.mtu_state = MtuState::Search;
    inner.mtu = MTU_BASE;
    inner.mtu_probe_size = MTU_BASE + MTU_STEP;

    inner.on_mtu_probe_acked();

    assert_eq!(inner.mtu, MTU_BASE + MTU_STEP);
    assert_eq!(inner.mtu_probe_count, 0);
    assert_eq!(inner.mtu_probe_size, MTU_BASE + 2 * MTU_STEP);
    assert!(inner.mtu_probe_wanted, "should want next probe");
    assert_eq!(
        inner.mtu_state,
        MtuState::Search,
        "still searching — haven't reached max"
    );
}

// T8: MTU_MAX_PROBES consecutive failures → SearchComplete
#[test]
fn mtu_max_probe_failures_completes_search() {
    let mut inner = make_stream_inner();
    inner.mtu_state = MtuState::Search;
    inner.mtu = MTU_BASE + MTU_STEP;

    for i in 0..MTU_MAX_PROBES {
        let payload = vec![0xEFu8; inner.max_payload()];
        let packet = build_data_packet(0, i, 0, &payload);
        inner.outgoing.insert(
            Seq(i),
            SentPacket {
                packet,
                time_sent: Instant::now() - Duration::from_millis(1),
                retransmit_count: 0,
                sacked: false,
                is_mtu_probe: true,
                lost: false,
                rate_info: None,
                remote_addr: None,
            },
        );

        let _ = rto_resend(&mut inner);
    }

    assert_eq!(inner.mtu_state, MtuState::SearchComplete);
    assert!(!inner.mtu_probe_wanted);
    assert_eq!(
        inner.mtu,
        MTU_BASE + MTU_STEP,
        "MTU frozen at last successful value"
    );
}

fn v4() -> Option<SocketAddr> {
    Some("127.0.0.1:9".parse().unwrap())
}

fn v6() -> Option<SocketAddr> {
    Some("[::1]:9".parse().unwrap())
}

#[test]
fn payload_at_base_mtu_matches_reference() {
    let mut inner = make_stream_inner();
    inner.remote_addr = v4();
    assert_eq!(inner.max_payload(), 1152);
}

#[test]
fn payload_for_ipv6_subtracts_sixty_eight() {
    let mut inner = make_stream_inner();
    inner.remote_addr = v6();
    assert_eq!(inner.max_payload(), 1200 - 68);
}

#[test]
fn payload_without_remote_addr_takes_the_bigger_overhead() {
    let mut inner = make_stream_inner();
    inner.remote_addr = None;
    assert_eq!(inner.max_payload(), 1200 - 68);
}

#[test]
fn probe_target_is_the_ip_packet_size() {
    let mut inner = make_stream_inner();
    inner.remote_addr = v4();
    let payload = vec![0u8; inner.max_payload()];
    let mut packet = build_data_packet(0, 0, 0, &payload);
    assert!(inner.mtu_probeify_packet(&mut packet));
    assert_eq!(packet.len() + 28, inner.mtu_probe_size);
    assert_eq!(packet[3] as usize, MTU_STEP);
}

#[test]
fn probe_target_for_ipv6_counts_forty_eight() {
    let mut inner = make_stream_inner();
    inner.remote_addr = v6();
    let payload = vec![0u8; inner.max_payload()];
    let mut packet = build_data_packet(0, 0, 0, &payload);
    assert!(inner.mtu_probeify_packet(&mut packet));
    assert_eq!(packet.len() + 48, inner.mtu_probe_size);
    assert_eq!(packet[3] as usize, MTU_STEP);
}

#[test]
fn mss_follows_the_payload_after_probe_ack() {
    let mut inner = make_stream_inner();
    inner.remote_addr = v4();
    inner.mtu_state = MtuState::Search;
    inner.on_mtu_probe_acked();
    // the controller's mss is a private field; indirectly: the stream payload that was passed in
    assert_eq!(inner.max_payload(), inner.mtu - 48);
}

#[test]
fn apply_link_mtu_none_keeps_max_and_state() {
    let mut inner = make_stream_inner();
    inner.apply_link_mtu(None);
    assert_eq!(inner.mtu_max, MTU_MAX);
    assert_eq!(inner.mtu_state, MtuState::Base);
}

#[test]
fn apply_link_mtu_above_max_is_clamped_to_max() {
    let mut inner = make_stream_inner();
    inner.apply_link_mtu(Some(9000));
    assert_eq!(inner.mtu_max, MTU_MAX);
    assert_eq!(inner.mtu_state, MtuState::Base);
}

#[test]
fn apply_link_mtu_1500_and_1452_set_max_keep_state() {
    for m in [1500usize, 1452] {
        let mut inner = make_stream_inner();
        inner.apply_link_mtu(Some(m));
        assert_eq!(inner.mtu_max, m);
        assert_eq!(inner.mtu_state, MtuState::Base);
    }
}

#[test]
fn apply_link_mtu_1200_completes_search() {
    let mut inner = make_stream_inner();
    inner.apply_link_mtu(Some(1200));
    assert_eq!(inner.mtu_max, 1200);
    assert_eq!(inner.mtu_state, MtuState::SearchComplete);
}

#[test]
fn apply_link_mtu_below_base_keeps_mtu_at_base() {
    let mut inner = make_stream_inner();
    inner.apply_link_mtu(Some(1000));
    assert_eq!(inner.mtu_max, 1000);
    assert_eq!(inner.mtu_state, MtuState::SearchComplete);
    assert_eq!(inner.mtu, MTU_BASE);
}

#[test]
fn apply_link_mtu_above_base_does_not_touch_search_state() {
    let mut inner = make_stream_inner();
    inner.mtu_state = MtuState::Search;
    inner.apply_link_mtu(Some(1452));
    assert_eq!(inner.mtu_state, MtuState::Search);
}
