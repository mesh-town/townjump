use super::*;

#[test]
fn test_new_does_not_panic() {
    let controller = CongestionController::new(1200, 50);
    assert_eq!(controller.ca_state, CaState::Open);
    assert_eq!(controller.cwnd, CongestionController::INITIAL_CWND);
    assert_eq!(controller.ssthresh, CongestionController::INITIAL_SSTHRESH);
}

#[test]
fn test_can_send_default_true() {
    let controller = CongestionController::new(1200, 50);
    assert!(controller.can_send(0, 4_194_304));
}

#[test]
fn test_pacing_delay_default_zero() {
    let mut controller = CongestionController::new(1200, 50);
    assert_eq!(
        controller.pacing_delay(1200, Instant::now()),
        Duration::ZERO
    );
}

#[test]
fn test_full_send_ack_cycle() {
    let mut c = CongestionController::new(1180, 100);
    let now = Instant::now();
    let rate_info = c.on_packet_sent(1, 0, now);
    let later = now + std::time::Duration::from_millis(10);
    c.on_ack(&[(1, rate_info)], 0, 0, later);
    assert!(c.cwnd >= BBR_CWND_MIN_TARGET);
    assert!(c.can_send(0, 4_194_304));
    assert!(!c.can_send(c.cwnd, 4_194_304));
}

#[test]
fn test_rwnd_in_packets_table() {
    assert_eq!(rwnd_in_packets(0, 1180), 0);
    assert_eq!(rwnd_in_packets(1, 1180), 1);
    assert_eq!(rwnd_in_packets(4096, 1180), 3);
}

#[test]
fn test_can_send_partial_window_allows_one_packet() {
    let controller = CongestionController::new(1180, 50);
    assert!(controller.can_send(0, 1));
    assert!(!controller.can_send(1, 1));
    assert!(!controller.can_send(0, 0));
}

/// Leaving Recovery compares sequence numbers modulo 2^32: the ACK of packet 1 after wraparound is later than high_seq = MAX.
#[test]
fn recovery_exit_compares_seq_across_the_wrap() {
    let mut c = CongestionController::new(1180, 100);
    let now = Instant::now();
    let rate_info = c.on_packet_sent(1, 0, now);
    c.ca_state = CaState::Recovery;
    c.high_seq = u32::MAX;
    c.on_ack(&[(1, rate_info)], 0, 0, now + Duration::from_millis(10));
    assert_eq!(c.ca_state, CaState::Open);
}

#[test]
fn recovery_stays_until_boundary_is_acked_across_the_wrap() {
    let mut c = CongestionController::new(1180, 100);
    let now = Instant::now();
    let rate_info = c.on_packet_sent(u32::MAX - 1, 0, now);
    c.ca_state = CaState::Recovery;
    c.high_seq = 1;
    c.on_ack(
        &[(u32::MAX - 1, rate_info)],
        0,
        0,
        now + Duration::from_millis(10),
    );
    assert_eq!(c.ca_state, CaState::Recovery);
}

/// Speed guard: a packet goes out while the bucket has any remainder at all (C `stream_may_send`).
/// 25 000 B of initial headroom with 1200 B packets gives 21 packets, the last one leaves with a remainder of 1000.
/// A bucket capped at 2·mss that rejects when the remainder is below one packet would let only one packet through per millisecond.
#[test]
fn pacing_admits_packet_on_any_leftover_allowance() {
    let mut c = CongestionController::new(1180, 100);
    let t = Instant::now();
    c.on_transmit_start(t);
    c.pacing.set_rate(100);

    let mut sent = 0;
    while sent < 100 && c.pacing_delay(1200, t).is_zero() {
        c.pacing.try_consume(1200, t);
        sent += 1;
    }
    assert_eq!(sent, 21);
}
