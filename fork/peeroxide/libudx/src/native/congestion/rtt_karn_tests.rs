// Karn protection for BBR's `rtt`: C `udx.c:1342-1344` (`rs->rtt_ms` only when `!retransmitted || rtt >= udx_rtt_min`)
// and a retransmission does not reset the flight start (`udx_rate.c:8-15`).
use super::*;

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// A controller whose BBR has already seen an RTT of 230 ms (`min_rtt` = 230).
fn with_min_rtt_230(t0: Instant) -> CongestionController {
    let mut c = CongestionController::new(1180, 100);
    let first = c.on_packet_sent(1, 0, t0);
    c.on_ack(&[(1, first)], 0, 0, t0 + ms(230));
    c.set_rtt_min(Some(230));
    assert_eq!(c.bbr().min_rtt_ms(), 230);
    c
}

/// Spurious retransmission: ACK 2 ms after the retransmission — the first copy actually arrived. `min_rtt` must not drop to 2 ms.
#[test]
fn disputed_resend_does_not_lower_min_rtt() {
    let t0 = Instant::now();
    let mut c = with_min_rtt_230(t0);
    let resent = c.on_packet_resent(t0 + ms(300));
    c.on_ack(&[(2, resent)], 0, 0, t0 + ms(302));
    assert_eq!(
        c.bbr().min_rtt_ms(),
        230,
        "retransmission with RTT 2 ms < min yields no measurement"
    );

    // Control: a genuine retransmission (RTT not shorter than min) is still accepted as a sample; a regular packet with RTT 2 ms lowers `min_rtt`.
    let mut c = with_min_rtt_230(t0);
    let resent = c.on_packet_resent(t0 + ms(300));
    c.on_ack(&[(2, resent)], 0, 0, t0 + ms(540));
    assert!(c.bbr().min_rtt_ms() <= 230);
    let mut c = with_min_rtt_230(t0);
    let plain = c.on_packet_sent(2, 0, t0 + ms(300));
    c.on_ack(&[(2, plain)], 0, 0, t0 + ms(302));
    assert_eq!(c.bbr().min_rtt_ms(), 2, "a regular packet lowers min_rtt");
}

/// A retransmission with an empty flight (everything lost) does not move `first_sent_ts`; a new send with an empty flight does.
#[test]
fn resend_with_empty_flight_keeps_first_sent_ts() {
    let t0 = Instant::now();
    let mut c = CongestionController::new(1180, 100);
    c.on_packet_sent(1, 0, t0);
    let info = c.on_packet_resent(t0 + ms(500));
    assert_eq!(info.first_sent_ts, t0);
    assert!(info.retransmitted);
    let fresh = c.on_packet_sent(3, 0, t0 + ms(800));
    assert_eq!(fresh.first_sent_ts, t0 + ms(800), "a new send resets it");
}
