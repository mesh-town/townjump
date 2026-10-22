//! Timer timings against JS libudx vectors (`testvectors/deltas/udx.json`, `results.new`, early tolerance 50 ms, lateness allowance growing per step on every system):
//! the zero-window probe (`zwp`) and RTO escalation (`rto`). The peer is a raw UDP socket, time is real.
#![deny(clippy::all)]

mod common;

use std::time::{Duration, Instant};

use common::raw_peer::{
    FLAG_DATA, FLAG_HEARTBEAT, Link, OPEN_WINDOW, RawPeer, Received, ack_packet, flags_of, seq_of,
};
use common::with_timeout;
use libudx::UdxError;

const TOLERANCE_MS: u64 = 50;
/// How much later than `TOLERANCE_MS` a timer may fire, `LATE_BASE_MS` plus `LATE_STEP_MS` per timer step before the
/// measured moment; applied on every system. Timers tick late (15.6 ms on Windows, hundreds of ms of scheduler delay
/// on a loaded or virtualized host), and every step re-arms the timer from `now()`, so lateness accumulates per
/// step: by the fourth step a timer can be 240 ms late. Timers never
/// fire early, so the early side stays at the plain `TOLERANCE_MS`. The smallest gap between two different schedules
/// (1 s vs 2 s intervals) is 1000 ms, far above the allowance (650 ms at step 7, the largest). This is the test's
/// tolerance only; the product keeps the JS schedule.
const LATE_BASE_MS: u64 = 250;
const LATE_STEP_MS: u64 = 50;
const TEST_LIMIT: Duration = Duration::from_secs(30);
/// ZWP probe length (jq: `.zwp[].len`).
const PROBE_LEN: usize = 20;

mod zero_window_probe {
    use super::*;

    mod peer_closed_the_window_before_write {
        use super::*;

        #[tokio::test]
        async fn probes_at_once_then_after_rto_then_after_rto_times_two() {
            with_timeout(TEST_LIMIT, async {
                // given: the peer announced window 0 and waited 100 ms (as in udxzwp.js)
                let Link {
                    mut peer,
                    stream,
                    keep_alive: _keep,
                } = Link::new().await;
                given_window_closed(&peer).await;
                // when: a write at a zero window
                peer.restart_clock();
                let _write = tokio::spawn(async move { stream.write(b"blocked").await });
                // the window outlasts the last probe (4 steps in) by the lateness the test allows
                let window = Duration::from_millis(7300 + late_allowance_ms(4));
                let packets = peer.collect_for(window).await;
                // then: jq .zwp[].approxMs = 100, 1100, 3100, 5100, 7100 -> from the first: 0/1000/3000/5000/7000
                let probes = matching(&packets, FLAG_HEARTBEAT);
                assert!(
                    probes
                        .first()
                        .is_some_and(|(ms, _)| *ms <= TOLERANCE_MS + late_allowance_ms(0)),
                    "first probe no later than {TOLERANCE_MS} ms after write: {probes:?}"
                );
                assert_offsets(&probes, &[0, 1000, 3000, 5000, 7000]);
                assert!(
                    probes.iter().all(|(_, p)| p.len() == PROBE_LEN),
                    "probe length"
                );
                assert!(
                    matching(&packets, FLAG_DATA).is_empty(),
                    "DATA does not go out at window 0"
                );
            })
            .await;
        }
    }

    mod window_opens_after_second_probe {
        use super::*;

        #[tokio::test]
        async fn data_follows_quickly_and_probes_stop() {
            with_timeout(TEST_LIMIT, async {
                // given: probes are running, the second is already received
                let Link {
                    mut peer,
                    stream,
                    keep_alive: _keep,
                } = Link::new().await;
                given_window_closed(&peer).await;
                let _write = tokio::spawn(async move { stream.write(b"blocked").await });
                recv_matching(&peer, FLAG_HEARTBEAT, 2).await;
                // when: the peer opened the window
                peer.restart_clock();
                peer.send(&ack_packet(0, OPEN_WINDOW, &[])).await;
                let packets = peer.collect_for(Duration::from_millis(2700)).await;
                // then: DATA <= 200 ms, no more probes
                let data = matching(&packets, FLAG_DATA);
                assert!(
                    data.first()
                        .is_some_and(|(ms, _)| *ms <= 200 + late_allowance_ms(0)),
                    "DATA no later than 200 ms after the window opened: {data:?}"
                );
                let probes = matching(&packets, FLAG_HEARTBEAT);
                assert!(
                    probes.is_empty(),
                    "no probes after the window opened: {probes:?}"
                );
            })
            .await;
        }
    }
}

mod silent_peer {
    use super::*;

    #[tokio::test]
    async fn data_is_retransmitted_on_rto_schedule_and_write_times_out() {
        with_timeout(TEST_LIMIT, async {
            // given: the peer is silent, the stream writes one packet
            let Link {
                mut peer,
                stream,
                keep_alive: _keep,
            } = Link::new().await;
            peer.restart_clock();
            let start = Instant::now();
            let write = tokio::spawn(async move {
                let result = stream.write(b"hello").await;
                (result, start.elapsed())
            });
            // when
            let packets = peer.collect_for(Duration::from_millis(13_500)).await;
            let (result, elapsed) = write.await.expect("write task");
            // then: jq .rto.packets[].approxMs = 0,1000,3000,5000,7000,9000,11000 (all seq 0)
            let data = matching(&packets, FLAG_DATA);
            assert_offsets(&data, &[0, 1000, 3000, 5000, 7000, 9000, 11_000]);
            assert!(
                data.iter().all(|(_, p)| seq_of(p) == 0),
                "all retransmissions are seq 0"
            );
            // and: jq .rto.events[0] = { approxMs: 13000, error: ETIMEDOUT }
            let Err(UdxError::Io(error)) = result else {
                panic!("write must fail with Io(TimedOut), not {result:?}");
            };
            assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
            let steps_before_failure = 7;
            assert_within(
                elapsed.as_millis() as u64,
                13_000,
                steps_before_failure,
                "write failed",
            );
        })
        .await;
    }
}

// ── setup and helpers ────────────────────────────────────────────────────────

/// The peer announces window 0 (an ACK without blocks) and waits 100 ms for the stream to take it into account.
async fn given_window_closed(peer: &RawPeer) {
    peer.send(&ack_packet(0, 0, &[])).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
}

/// Packets with flag `flag` out of the collected ones.
fn matching(packets: &[Received], flag: u8) -> Vec<Received> {
    packets
        .iter()
        .filter(|(_, packet)| flags_of(packet) & flag != 0)
        .cloned()
        .collect()
}

/// Waits for `count` packets with flag `flag` (each up to 3 s), skipping the others.
async fn recv_matching(peer: &RawPeer, flag: u8, count: usize) {
    let mut seen = 0;
    while seen < count {
        let (_, packet) = peer
            .recv_within(Duration::from_secs(3))
            .await
            .unwrap_or_else(|| panic!("packet with flag {flag:#x} did not arrive within 3 s"));
        seen += usize::from(flags_of(&packet) & flag != 0);
    }
}

/// Packet times from the first equal `expected` (ms) within tolerance; the packet count matches.
fn assert_offsets(packets: &[Received], expected: &[u64]) {
    let first = packets.first().map_or(0, |(ms, _)| *ms);
    let actual: Vec<u64> = packets.iter().map(|(ms, _)| ms - first).collect();
    assert_eq!(
        actual.len(),
        expected.len(),
        "packet count, times: {actual:?}"
    );
    for (index, (got, want)) in actual.iter().zip(expected).enumerate() {
        assert_within(
            *got,
            *want,
            index,
            &format!("packet no. {index} (all times {actual:?})"),
        );
    }
}

/// How much later than `TOLERANCE_MS` a timer may fire after `step` timer steps.
fn late_allowance_ms(step: usize) -> u64 {
    LATE_BASE_MS + LATE_STEP_MS * step as u64
}

/// `step` is the number of timer steps before the measured moment (0 for the first packet).
fn assert_within(got_ms: u64, want_ms: u64, step: usize, what: &str) {
    let late = TOLERANCE_MS + late_allowance_ms(step);
    assert!(
        got_ms + TOLERANCE_MS >= want_ms && got_ms <= want_ms + late,
        "{what}: {got_ms} ms, expected {want_ms} (-{TOLERANCE_MS} / +{late})"
    );
}
