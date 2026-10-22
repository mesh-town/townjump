//! F1 (on by default, `UDX_DIAG=nof1` reverts it): leave recovery by the header's cumulative ACK, not by SACK number. We do not touch env.
use super::test_support::{stream_inner, with_sent};
use super::*;
use std::time::Duration;

/// A controller in `Recovery` with boundary `high_seq`; one packet `seq` is acked as a "SACK" via `on_ack`.
fn on_ack_at_boundary(sack_exit: bool, high_seq: u32) -> CaState {
    let mut c = congestion::CongestionController::new(1180, 100);
    let now = Instant::now();
    let info = c.on_packet_sent(high_seq, 0, now);
    c.sack_exit = sack_exit;
    c.ca_state = CaState::Recovery;
    c.high_seq = high_seq;
    c.on_ack(&[(high_seq, info)], 0, 0, now + Duration::from_millis(10));
    c.ca_state
}

#[test]
fn sack_exit_off_keeps_recovery_at_sack_number_equal_to_high_seq() {
    assert_eq!(on_ack_at_boundary(false, 7), CaState::Recovery);
}

#[test]
fn sack_exit_on_leaves_recovery_at_sack_number_equal_to_high_seq() {
    assert_eq!(on_ack_at_boundary(true, 7), CaState::Open);
}

fn exit_state(ack: u32, high_seq: u32) -> CaState {
    let mut inner = stream_inner();
    inner.congestion.ca_state = CaState::Recovery;
    inner.congestion.high_seq = high_seq;
    inner.f1_exit_by_cum_ack(ack);
    inner.congestion.ca_state
}

#[test]
fn cum_ack_equal_to_high_seq_stays_in_recovery() {
    assert_eq!(exit_state(9, 9), CaState::Recovery);
}

#[test]
fn cum_ack_one_past_high_seq_exits() {
    assert_eq!(exit_state(10, 9), CaState::Open);
}

#[test]
fn cum_ack_boundary_holds_across_the_2_pow_32_wrap() {
    assert_eq!(exit_state(u32::MAX, u32::MAX), CaState::Recovery);
    assert_eq!(exit_state(0, u32::MAX), CaState::Open);
    assert_eq!(exit_state(u32::MAX, u32::MAX - 1), CaState::Open);
}

#[test]
fn cum_ack_exit_ignores_open_state() {
    let mut inner = stream_inner();
    inner.congestion.high_seq = 3;
    inner.f1_exit_by_cum_ack(100);
    assert_eq!(inner.congestion.ca_state, CaState::Open);
}

/// A document, not a fix: `high_seq` in rs is the highest `outgoing` key (N-1), in C it is `nxt` (N). The port leaves one number earlier:
/// At `ack = N` C (`ack > nxt`) still holds recovery, while rs with F1 (`ack > N-1`) is already in Open.
#[test]
fn f1_high_seq_is_one_number_earlier_than_c() {
    let mut inner = stream_inner();
    with_sent(&mut inner, 0, 10); // sent 0..9, next_seq = 10 (in C nxt = 10)
    inner.on_sack(&[SackRange { start: 5, end: 10 }]);
    assert_eq!(inner.detect_losses(), 5);
    assert_eq!(inner.congestion.ca_state, CaState::Recovery);
    assert_eq!(
        inner.congestion.high_seq, 9,
        "rs: highest outgoing key, N-1"
    );
    assert_eq!(inner.congestion.high_seq, inner.next_seq - 1);
    // ack = 10 = C `nxt`: C holds recovery (10 > 10 is false), rs leaves (10 > 9).
    inner.f1_exit_by_cum_ack(9);
    assert_eq!(inner.congestion.ca_state, CaState::Recovery);
    inner.f1_exit_by_cum_ack(10);
    assert_eq!(inner.congestion.ca_state, CaState::Open);
}

/// End to end: by default (`parse("")` gives f1) `apply_acks` disables the SACK exit, and a cumulative ACK leaves.
/// Env is read once, so the default is checked via `parse("")`; the test itself runs without `UDX_DIAG=nof1`.
#[test]
fn default_mode_exits_recovery_only_by_cumulative_ack() {
    assert!(diag::parse("").f1);
    assert!(diag::mode().f1, "run the test without UDX_DIAG=nof1");
    let mut inner = stream_inner();
    with_sent(&mut inner, 0, 12);
    inner.on_sack(&[SackRange { start: 5, end: 12 }]);
    assert!(inner.detect_losses() > 0);
    assert_eq!(inner.congestion.ca_state, CaState::Recovery);
    assert!(
        inner.congestion.sack_exit,
        "before the first ACK the flag is on by default"
    );
    // the SACK number reached high_seq, the cumulative ACK did not (0): recovery holds, the SACK exit is disabled
    let _ = inner.apply_acks(0, &[SackRange { start: 5, end: 12 }]);
    assert!(!inner.congestion.sack_exit);
    assert_eq!(inner.congestion.ca_state, CaState::Recovery);
    // cumulative ACK past high_seq — exit
    let _ = inner.apply_acks(12, &[]);
    assert_eq!(inner.congestion.ca_state, CaState::Open);
}
