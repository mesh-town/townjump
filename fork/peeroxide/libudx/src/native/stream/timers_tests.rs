//! Stream timer tests: RTO (D7) and the zero-window probe ZWP (D4). We do not wait for time — we check
//! the returned timers; real timings on the wire are in `tests/wire_timers.rs`.
use super::*;
use crate::error::{Result, UdxError};
use crate::native::header::{FLAG_HEARTBEAT, HEADER_SIZE, Header, SackRange};
use crate::native::stream::test_support::{
    stream_inner, with_pending_write, with_queued, with_sent,
};
use crate::native::stream::{DEFAULT_RWND, FAR_FUTURE, StreamInner};
use std::io::ErrorKind;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::sync::oneshot::{self, error::TryRecvError};

mod timer_arithmetic {
    use super::*;

    #[test]
    fn single_is_the_rto_and_double_is_twice_the_rto() {
        assert_eq!((single_rto(1500), double_rto(1500)), (ms(1500), ms(3000)));
    }
}

mod rto_fires {
    use super::*;

    mod with_one_unacked_packet {
        use super::*;

        #[test]
        fn six_timeouts_restart_the_timer_at_double_rto_and_rto_does_not_grow() {
            // given: seq 0 sent, rto = 1000
            let mut sending = sending();
            // when
            let timers: Vec<_> = (0..6).map(|_| sending.inner.on_rto_fire()).collect();
            // then
            assert_eq!(timers, vec![Some(ms(2000)); 6]);
            assert_eq!(sending.inner.rto, 1000, "rto does not grow");
        }

        #[test]
        fn rto_1500_gives_timer_of_3000() {
            // given: rto 1500
            let mut sending = sending();
            sending.inner.rto = 1500;
            // when
            let timer = sending.inner.on_rto_fire();
            // then
            assert_eq!(timer, Some(ms(3000)));
        }

        #[test]
        fn write_stays_pending_through_six_timeouts() {
            // given
            let mut sending = sending();
            // when
            fire(&mut sending.inner, 6);
            // then
            let state = sending.write.try_recv();
            assert!(matches!(state, Err(TryRecvError::Empty)), "{state:?}");
        }

        #[test]
        fn seventh_timeout_asks_to_close_the_stream() {
            // given: six firings have already happened
            let mut sending = sending();
            fire(&mut sending.inner, 6);
            // when
            let timer = sending.inner.on_rto_fire();
            // then
            assert_eq!(timer, None);
        }

        #[test]
        fn seventh_timeout_fails_the_pending_write_with_timed_out() {
            // given
            let mut sending = sending();
            fire(&mut sending.inner, 6);
            // when
            sending.inner.on_rto_fire();
            // then
            match sending.write.try_recv() {
                Ok(Err(UdxError::Io(error))) => assert_eq!(error.kind(), ErrorKind::TimedOut),
                other => panic!("expected Err(Io(TimedOut)), got {other:?}"),
            }
        }
    }

    mod with_nothing_in_flight {
        use super::*;

        #[test]
        fn timer_goes_to_far_future() {
            // given: outgoing is empty
            let mut inner = stream_inner();
            // when
            let timer = inner.on_rto_fire();
            // then
            assert_eq!(timer, Some(FAR_FUTURE));
        }

        #[test]
        fn timeout_counter_does_not_grow() {
            // given
            let mut inner = stream_inner();
            // when
            fire(&mut inner, 3);
            // then
            assert_eq!(inner.rto_timeouts, 0);
        }
    }
}

mod rto_timeout_counter {
    use super::*;

    #[test]
    fn ack_that_advances_resets_it() {
        // given: seq 0 sent, three firings
        let mut sending = sending();
        fire(&mut sending.inner, 3);
        assert_eq!(sending.inner.rto_timeouts, 3, "precondition");
        // when: ack 1 confirmed the packet
        sending.inner.on_ack_packet(1, DEFAULT_RWND, &[]);
        // then
        assert_eq!(sending.inner.rto_timeouts, 0);
    }

    #[test]
    fn sack_without_advance_keeps_it() {
        // given: seq 0 and 1 sent, three firings
        let mut inner = stream_inner();
        with_sent(&mut inner, 0, 2);
        fire(&mut inner, 3);
        // when: only a SACK on seq 1, ack does not move
        inner.on_ack_packet(0, DEFAULT_RWND, &[SackRange { start: 1, end: 2 }]);
        // then
        assert_eq!(inner.rto_timeouts, 3);
    }
}

mod rtt_sample {
    use super::*;

    #[test]
    fn zero_millis_is_measured_as_one() {
        // given: no measurements yet
        let mut inner = stream_inner();
        // when
        inner.update_rtt(0);
        // then
        assert_eq!(inner.srtt, 1);
    }
}

mod zero_window_probe {
    use super::*;

    mod window_zero_with_queued_data_and_idle_send {
        use super::*;

        #[test]
        fn write_starts_probing_with_timer_of_one_rto() {
            // given
            let mut inner = blocked_stream();
            // when
            let timer = inner.zwp_start_on_write();
            // then
            assert_eq!(timer, Some(ms(1000)));
            assert!(inner.zwp_active, "ZWP on");
        }

        #[test]
        fn second_start_does_nothing() {
            // given: ZWP already started
            let mut inner = blocked_stream();
            inner.zwp_start_on_write();
            // when
            let timer = inner.zwp_start_on_write();
            // then
            assert_eq!(timer, None);
        }

        #[test]
        fn each_fire_restarts_timer_at_double_rto() {
            // given
            let mut inner = blocked_stream();
            inner.zwp_start_on_write();
            // when
            let timers = [inner.on_zwp_fire(), inner.on_zwp_fire()];
            // then
            assert_eq!(timers, [ms(2000), ms(2000)]);
        }

        #[test]
        fn probes_come_at_0_1000_3000_5000_ms() {
            // given: a probe right at start
            let mut inner = blocked_stream();
            let mut probe_at = ms(0);
            let mut times = vec![probe_at];
            // when: every returned timer is a wait until the next probe
            probe_at += inner.zwp_start_on_write().expect("start");
            times.push(probe_at);
            for _ in 0..2 {
                probe_at += inner.on_zwp_fire();
                times.push(probe_at);
            }
            // then
            assert_eq!(times, [ms(0), ms(1000), ms(3000), ms(5000)]);
        }

        #[test]
        fn window_opening_stops_probing() {
            // given: ZWP is running
            let mut inner = blocked_stream();
            inner.zwp_start_on_write();
            // when: the window arrived
            inner.send_rwnd = DEFAULT_RWND;
            let timer = inner.zwp_stop_on_window();
            // then
            assert_eq!(timer, Some(FAR_FUTURE));
            assert!(!inner.zwp_active, "ZWP off");
        }

        #[test]
        fn fire_after_stop_returns_far_future() {
            // given: the window opened, ZWP cleared
            let mut inner = blocked_stream();
            inner.zwp_start_on_write();
            inner.send_rwnd = DEFAULT_RWND;
            inner.zwp_stop_on_window();
            // when
            let timer = inner.on_zwp_fire();
            // then
            assert_eq!(timer, FAR_FUTURE);
        }

        #[test]
        fn fire_with_window_already_open_turns_zwp_off() {
            // given: the window opened, but ZWP was not cleared in time
            let mut inner = blocked_stream();
            inner.zwp_start_on_write();
            inner.send_rwnd = DEFAULT_RWND;
            // when
            let timer = inner.on_zwp_fire();
            // then
            assert_eq!(timer, FAR_FUTURE);
            assert!(!inner.zwp_active, "ZWP off");
        }

        #[test]
        fn window_still_zero_does_not_stop_probing() {
            // given
            let mut inner = blocked_stream();
            inner.zwp_start_on_write();
            // when
            let timer = inner.zwp_stop_on_window();
            // then
            assert_eq!(timer, None);
            assert!(inner.zwp_active, "ZWP continues");
        }
    }

    mod no_reason_to_probe {
        use super::*;

        #[test]
        fn packets_in_flight_prevent_start() {
            // given: window 0, queue not empty, but outgoing not empty
            let mut inner = blocked_stream();
            with_sent(&mut inner, 0, 1);
            // when
            let timer = inner.zwp_start_on_write();
            // then
            assert_eq!(timer, None);
            assert!(!inner.zwp_active, "ZWP is not active");
        }

        #[test]
        fn empty_queue_prevents_start() {
            // given: window 0, but nothing to send
            let mut inner = stream_inner();
            inner.send_rwnd = 0;
            // when
            let timer = inner.zwp_start_on_write();
            // then
            assert_eq!(timer, None);
        }

        #[test]
        fn open_window_prevents_start() {
            // given: data in the queue, window open
            let mut inner = stream_inner();
            with_queued(&mut inner, 0);
            // when
            let timer = inner.zwp_start_on_write();
            // then
            assert_eq!(timer, None);
        }
    }

    mod probe_on_the_wire {
        use super::*;

        #[tokio::test]
        async fn start_sends_heartbeat_with_queue_head_seq_and_remote_progress() {
            // given: seq 4 in the queue with next_seq 6, accepted up to seq 3
            let mut wired = wired_blocked_stream().await;
            // when
            wired.inner.zwp_start_on_write();
            // then
            let packet = wired.received().await;
            let header = Header::decode(&packet).unwrap();
            assert_eq!(packet.len(), HEADER_SIZE, "probe is 20 bytes");
            assert_eq!(header.type_flags, FLAG_HEARTBEAT);
            assert_eq!((header.remote_id, header.seq, header.ack), (7, 4, 3));
            assert_eq!(wired.inner.next_seq, 6, "seq is not consumed by the probe");
        }

        #[tokio::test]
        async fn fire_sends_another_heartbeat() {
            // given: ZWP started, the first probe read
            let mut wired = wired_blocked_stream().await;
            wired.inner.zwp_start_on_write();
            wired.received().await;
            // when
            wired.inner.on_zwp_fire();
            // then
            let header = Header::decode(&wired.received().await).unwrap();
            assert_eq!(header.type_flags, FLAG_HEARTBEAT);
            assert_eq!(header.seq, 4);
        }
    }
}

// ── setup and helpers ────────────────────────────────────────────────────────

fn ms(millis: u64) -> Duration {
    Duration::from_millis(millis)
}

/// A stream with one sent packet (seq 0) and a write pending on it.
struct Sending {
    inner: StreamInner,
    write: oneshot::Receiver<Result<()>>,
}

fn sending() -> Sending {
    let mut inner = stream_inner();
    with_sent(&mut inner, 0, 1);
    let write = with_pending_write(&mut inner, 1);
    Sending { inner, write }
}

/// `times` RTO firings in a row.
fn fire(inner: &mut StreamInner, times: u32) {
    for _ in 0..times {
        inner.on_rto_fire();
    }
}

/// Window 0, an unsent seq 4 in the queue with `next_seq` 6, `outgoing` empty, accepted up to seq 3.
fn blocked_stream() -> StreamInner {
    let mut inner = stream_inner();
    inner.send_rwnd = 0;
    inner.next_seq = 6;
    inner.next_remote_seq = 3;
    with_queued(&mut inner, 4);
    inner
}

/// `blocked_stream` with a real UDP socket and a peer that reads what was sent.
struct Wired {
    inner: StreamInner,
    peer: UdpSocket,
}

async fn wired_blocked_stream() -> Wired {
    let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let own = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let mut inner = blocked_stream();
    inner.remote_id = 7;
    inner.remote_addr = Some(peer.local_addr().unwrap());
    inner.udp = Some(Arc::new(own));
    Wired { inner, peer }
}

impl Wired {
    /// The first packet that reached the peer (within 1 s).
    async fn received(&self) -> Vec<u8> {
        let mut buf = [0u8; 2048];
        let read = tokio::time::timeout(Duration::from_secs(1), self.peer.recv_from(&mut buf));
        let (len, _) = read
            .await
            .expect("packet did not arrive within 1 s")
            .unwrap();
        buf[..len].to_vec()
    }
}
