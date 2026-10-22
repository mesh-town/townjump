// Tests of the delivery-sample interval and sample RTT: C `udx_rate.c:49-51` — `snd_ms = pkt->time_sent - pkt->first_sent_ts`,
// C `udx.c:1305` — `rtt = time - pkt->time_sent`. Computing both from `info.first_sent_ts` would make `snd_ms` always 0.
use super::*;
use std::collections::VecDeque;

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// The sample interval is `max(snd_ms, ack_ms)`. X is acked after 200 ms, Y was sent almost right before the ack of the pair {Z, Y}:
/// the ack span of Y is 200 ms (from the ack of X), the send span is 390 ms (from sending X to sending Y).
/// With `snd_ms = 0` the interval would come out as 200 ms and the rate would be overestimated almost twofold.
#[test]
fn sample_interval_is_the_send_phase_when_it_is_longer_than_the_ack_phase() {
    let mut c = CongestionController::new(1180, 100);
    let t0 = Instant::now();
    let x = c.on_packet_sent(1, 0, t0);
    let z = c.on_packet_sent(2, 1, t0 + ms(5));
    c.on_ack(&[(1, x)], 0, 1, t0 + ms(200));
    let y = c.on_packet_sent(3, 1, t0 + ms(390));
    c.on_ack(&[(2, z), (3, y)], 0, 0, t0 + ms(400));
    let rs = &c.rate.current;
    assert_eq!(rs.delivered, 2);
    assert_eq!(
        rs.interval_ms, 390,
        "interval = send span, not just the ack span"
    );
}

/// Sample RTT is measured from sending the acked packet itself (Y was sent 10 ms before the ack), not from the start of the burst.
#[test]
fn rtt_of_a_sample_is_counted_from_the_send_time_of_the_acked_packet() {
    let mut c = CongestionController::new(1180, 100);
    let t0 = Instant::now();
    let x = c.on_packet_sent(1, 0, t0);
    let z = c.on_packet_sent(2, 1, t0 + ms(5));
    c.on_ack(&[(1, x)], 0, 1, t0 + ms(200));
    let y = c.on_packet_sent(3, 1, t0 + ms(390));
    c.on_ack(&[(2, z), (3, y)], 0, 0, t0 + ms(400));
    assert_eq!(c.rate.current.rtt_ms, 10);
}

/// Continuous sending every 10 ms, the pipeline never empties, ack after 250 ms: sample RTT is always 250 ms.
/// If `first_sent_ts` froze at the start of the connection, sample RTT would grow with the connection age
/// (`minrtt` far above the true 0.26 s), and with it the target window `bw × min_rtt`.
#[test]
fn rtt_of_a_sample_does_not_grow_with_the_age_of_the_connection() {
    let mut c = CongestionController::new(1180, 100);
    let t0 = Instant::now();
    let mut pending: VecDeque<(Instant, u32, rate::PacketRateInfo)> = VecDeque::new();
    let mut worst = 0;
    for k in 0..200u64 {
        let now = t0 + ms(10 * k);
        while pending.front().is_some_and(|(ack_at, _, _)| *ack_at <= now) {
            let (ack_at, seq, info) = pending.pop_front().unwrap();
            c.on_ack(&[(seq, info)], 0, pending.len() as u32, ack_at);
            worst = worst.max(c.rate.current.rtt_ms);
        }
        let seq = k as u32 + 1;
        let info = c.on_packet_sent(seq, pending.len() as u32, now);
        pending.push_back((now + ms(250), seq, info));
    }
    assert_eq!(worst, 250, "sample RTT is not above the true 250 ms");
}
