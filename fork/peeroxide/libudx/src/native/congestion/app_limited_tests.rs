// Tests of the app-limited flag and window growth: C `udx__rate_gen` compares `delivered` and `app_limited` signed with wraparound.
use super::*;
use std::collections::VecDeque;

fn ack_after(c: &mut CongestionController, sent: &[(u32, rate::PacketRateInfo)], at: Instant) {
    c.on_ack(sent, 0, 0, at);
}

/// The stream starts app-limited (`~0`, as in C) and must leave it on the first ack:
/// in C `seq_compare(1, ~0) = +2`. Unsigned `1 > u32::MAX` is never true, so the flag would stay set for the life of the connection.
#[test]
fn app_limited_clears_on_first_ack() {
    let mut c = CongestionController::new(1180, 100);
    let now = Instant::now();
    let first = c.on_packet_sent(1, 0, now);
    assert!(first.is_app_limited, "app-limited before the first ACK");
    ack_after(&mut c, &[(1, first)], now + Duration::from_millis(10));
    assert_eq!(c.app_limited, 0);
}

/// After the flag is cleared packets go out non-app-limited: a sample taken from such a packet moves the bandwidth filter and the full_bw count.
#[test]
fn packets_sent_after_first_ack_are_not_stamped_app_limited() {
    let mut c = CongestionController::new(1180, 100);
    let now = Instant::now();
    let first = c.on_packet_sent(1, 0, now);
    ack_after(&mut c, &[(1, first)], now + Duration::from_millis(10));
    c.check_app_limited(10_000_000, 0);
    let second = c.on_packet_sent(2, 0, now + Duration::from_millis(11));
    assert!(!second.is_app_limited);
}

/// Once the queue runs empty the flag is set again: `delivered + inflight`, and cleared when `delivered` passes this mark.
#[test]
fn app_limited_is_set_on_empty_queue_and_cleared_past_the_mark() {
    let mut c = CongestionController::new(1180, 100);
    let now = Instant::now();
    c.app_limited = 0;
    c.delivered = 2;
    c.check_app_limited(0, 3);
    assert_eq!(c.app_limited, 5);
    let infos: Vec<_> = (1..=3)
        .map(|s| (s, c.on_packet_sent(s, s - 1, now)))
        .collect();
    ack_after(&mut c, &infos, now + Duration::from_millis(10));
    assert_eq!(c.delivered, 5);
    assert_eq!(
        c.app_limited, 5,
        "delivered == mark: strictly greater, the flag stays"
    );
    let more = c.on_packet_sent(4, 0, now + Duration::from_millis(11));
    ack_after(&mut c, &[(4, more)], now + Duration::from_millis(21));
    assert_eq!(c.app_limited, 0);
}

/// Lossless bottleneck: a FIFO queue at `rate` packets per ms and `rtt_ms` delay on ACK.
struct Link {
    rate: f64,
    rtt_ms: f64,
    free_at: f64,
    pending: VecDeque<(u32, rate::PacketRateInfo, f64)>,
}

impl Link {
    fn send(&mut self, seq: u32, info: rate::PacketRateInfo, at_ms: f64) {
        self.free_at = self.free_at.max(at_ms) + 1.0 / self.rate;
        self.pending
            .push_back((seq, info, self.free_at + self.rtt_ms));
    }

    fn acked_by(&mut self, at_ms: f64) -> Vec<(u32, rate::PacketRateInfo)> {
        let mut batch = Vec::new();
        while self.pending.front().is_some_and(|p| p.2 <= at_ms) {
            let (seq, info, _) = self.pending.pop_front().unwrap();
            batch.push((seq, info));
        }
        batch
    }
}

/// Run result: how much was delivered, the window and the BBR mode at the end.
struct Run {
    delivered: u32,
    cwnd: u32,
    mode: BbrMode,
    full_bw_reached: bool,
    bw: f64,
}

/// Constant fill (the write queue is not empty) in 1 ms ticks: like the stack — `check_app_limited`, window, pacing, send.
fn bulk_run(rate: f64, rtt_ms: f64, run_ms: u64) -> Run {
    let mut c = CongestionController::new(1180, 100);
    let t0 = Instant::now();
    let mut link = Link {
        rate,
        rtt_ms,
        free_at: 0.0,
        pending: VecDeque::new(),
    };
    let mut seq = 0u32;
    for ms in 0..run_ms {
        let now = t0 + Duration::from_millis(ms);
        let batch = link.acked_by(ms as f64);
        if !batch.is_empty() {
            let inflight = link.pending.len() as u32;
            c.on_ack(&batch, 0, inflight, now);
        }
        loop {
            let inflight = link.pending.len() as u32;
            c.check_app_limited(10_000_000, inflight);
            if !c.can_send(inflight, 4_194_304) || !c.pacing_delay(1200, now).is_zero() {
                break;
            }
            seq += 1;
            let info = c.on_packet_sent(seq, inflight, now);
            c.pacing.try_consume(1200, now);
            link.send(seq, info, ms as f64);
        }
    }
    Run {
        delivered: c.delivered,
        cwnd: c.cwnd,
        mode: c.bbr.mode(),
        full_bw_reached: c.bbr.full_bw_reached(),
        bw: c.bbr.bw(),
    }
}

/// Clean path of 2 packets/ms (~19 Mbit/s at 1200 B), RTT 250 ms: BDP = 500 packets. If the flag stayed set, `check_full_bw_reached`
/// would skip every sample, BBR would stay in Startup and the window would grow without a ceiling; with the flag cleared BBR exits to ProbeBw.
#[test]
fn bbr_leaves_startup_and_bounds_cwnd_on_a_clean_path() {
    let r = bulk_run(2.0, 250.0, 8_000);
    assert!(
        r.delivered > 10_000,
        "model works: delivered {}",
        r.delivered
    );
    assert_ne!(r.mode, BbrMode::Startup);
    assert!(r.full_bw_reached, "bandwidth plateau found");
    assert!(r.bw > 0.0, "bandwidth estimate is positive: {}", r.bw);
    assert!(
        r.cwnd <= 4 * 500,
        "window is bounded by BDP·gain, not unbounded: {}",
        r.cwnd
    );
}

#[test]
fn seq_helpers_compare_across_the_wrap() {
    // Plain `>` would answer the opposite for every pair that straddles 2^32.
    assert!(seq_gt(1, u32::MAX));
    assert!(!seq_gt(u32::MAX, 1));
    assert!(seq_gt(2, u32::MAX - 1));
    assert!(seq_gt(3, 2));
    assert!(!seq_gt(5, 5));
    assert!(seq_ge(5, 5));
    assert!(seq_ge(0, u32::MAX));
    assert!(!seq_ge(u32::MAX, 0));
}
