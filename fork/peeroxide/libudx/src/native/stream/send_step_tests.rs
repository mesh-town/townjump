//! Sender step tests: choice of pacing re-arm, notification coalescing, a step on a live socket.
use super::*;
use crate::native::stream::test_support::stream_inner;
use std::net::UdpSocket as StdUdp;
use tokio::sync::mpsc::error::TryRecvError;

// ── setup and helpers ────────────────────────────────────────────────────────

/// A stream with `remote_id = 7` and a real socket, the destination is `peer`.
fn wired(peer: SocketAddr) -> StreamInner {
    let own = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    own.set_nonblocking(true).unwrap();
    let mut inner = stream_inner();
    inner.remote_id = 7;
    inner.remote_addr = Some(peer);
    inner.udp = Some(Arc::new(UdpSocket::from_std(own).unwrap()));
    inner
}

/// Queues `n` DATA with numbers `0..n`.
fn queue_n(inner: &mut StreamInner, n: u32, addr: SocketAddr) {
    let packets = (0..n)
        .map(|s| (s, build_data_packet(inner.remote_id, s, 0, b"q")))
        .collect();
    inner.queue_for_send(packets, addr);
}

fn shared(inner: StreamInner) -> Arc<Mutex<StreamInner>> {
    Arc::new(Mutex::new(inner))
}

fn peer_socket() -> (StdUdp, SocketAddr) {
    let peer = StdUdp::bind("127.0.0.1:0").unwrap();
    peer.set_nonblocking(true).unwrap();
    let addr = peer.local_addr().unwrap();
    (peer, addr)
}

// ── after_step ───────────────────────────────────────────────────────────────

#[test]
fn after_step_rearms_exactly_by_pacing_delay() {
    // given: rows (delay, has_more, shutting_down) -> expectation
    let d = Duration::from_micros(300);
    let far = ControlFlow::Continue(FAR_FUTURE);
    let rows = [
        ((Some(d), true, false), ControlFlow::Continue(d)),
        ((None, true, false), far),
        ((None, false, true), ControlFlow::Break(())),
        ((Some(d), true, true), ControlFlow::Continue(d)),
        ((None, false, false), far),
    ];
    for ((delay, more, down), want) in rows {
        // when, then
        assert_eq!(
            after_step(delay, more, down),
            want,
            "{delay:?} {more} {down}"
        );
    }
}

// ── drain_notifies ───────────────────────────────────────────────────────────

#[test]
fn drain_sees_shutdown_behind_data_queued_and_empties_channel() {
    // given: [Shutdown, DataQueued] in the channel
    let (tx, mut rx) = mpsc::unbounded_channel();
    tx.send(StreamNotify::Shutdown).unwrap();
    tx.send(StreamNotify::DataQueued).unwrap();
    // when
    let shutdown = drain_notifies(StreamNotify::DataQueued, &mut rx);
    // then
    assert!(shutdown);
    assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));
}

#[test]
fn drain_of_empty_channel_reports_no_shutdown() {
    // given: an empty channel
    let (_tx, mut rx) = mpsc::unbounded_channel();
    // when, then
    assert!(!drain_notifies(StreamNotify::DataQueued, &mut rx));
}

// ── send_step ────────────────────────────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn first_send_from_idle_arms_rto() {
    // given: one packet in the queue
    let (peer, addr) = peer_socket();
    let mut inner = wired(addr);
    queue_n(&mut inner, 1, addr);
    let rto = single_rto(inner.rto);
    let inner = shared(inner);
    let mut timers = LoopTimers::new();
    // when
    let flow = send_step(&inner, &mut timers, false).await;
    // then
    assert!(flow.is_continue());
    assert_eq!(timers.rto.deadline(), TokioInstant::now() + rto);
    let mut buf = [0u8; 2048];
    let arrived = (0..200).any(|_| {
        peer.recv_from(&mut buf).is_ok() || {
            std::thread::sleep(Duration::from_millis(1));
            false
        }
    });
    assert!(arrived, "packet at the peer");
}

#[tokio::test(start_paused = true)]
async fn zero_window_in_idle_starts_zwp_and_holds_data() {
    // given: peer window 0, one packet in the queue
    let (_peer, addr) = peer_socket();
    let mut inner = wired(addr);
    inner.send_rwnd = 0;
    queue_n(&mut inner, 1, addr);
    let rto = single_rto(inner.rto);
    let inner = shared(inner);
    let mut timers = LoopTimers::new();
    // when
    let _ = send_step(&inner, &mut timers, false).await;
    // then
    let guard = inner.lock().unwrap();
    assert!(guard.zwp_active);
    assert!(guard.outgoing.is_empty());
    assert_eq!(guard.send_queue.len(), 1);
    let now = TokioInstant::now();
    assert_eq!(timers.zwp.deadline(), now + rto);
    assert_eq!(timers.pacing.deadline(), now + FAR_FUTURE);
}

#[tokio::test(start_paused = true)]
async fn shutdown_with_empty_queue_breaks() {
    // given: the queue is empty
    let (_peer, addr) = peer_socket();
    let inner = shared(wired(addr));
    let mut timers = LoopTimers::new();
    // when, then
    assert!(send_step(&inner, &mut timers, true).await.is_break());
}

#[tokio::test(start_paused = true)]
async fn shutdown_with_queue_left_continues() {
    // given: the window is closed by cwnd (11 packets at cwnd 10), shutdown in progress
    let (_peer, addr) = peer_socket();
    let mut inner = wired(addr);
    queue_n(&mut inner, 11, addr);
    let inner = shared(inner);
    let mut timers = LoopTimers::new();
    // when
    let flow = send_step(&inner, &mut timers, true).await;
    // then
    assert!(flow.is_continue());
    assert_eq!(inner.lock().unwrap().send_queue.len(), 1);
}

#[tokio::test(start_paused = true)]
async fn window_closed_by_cwnd_leaves_pacing_idle() {
    // given: 11 packets at cwnd 10
    let (_peer, addr) = peer_socket();
    let mut inner = wired(addr);
    queue_n(&mut inner, 11, addr);
    let inner = shared(inner);
    let mut timers = LoopTimers::new();
    // when
    let flow = send_step(&inner, &mut timers, false).await;
    // then: an ACK will open the window, no timer needed
    assert!(flow.is_continue());
    let guard = inner.lock().unwrap();
    assert_eq!(guard.outgoing.len(), 10);
    assert_eq!(guard.send_queue.len(), 1);
    assert_eq!(timers.pacing.deadline(), TokioInstant::now() + FAR_FUTURE);
}

// ── notify_step ──────────────────────────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn notify_from_idle_sends_at_once() {
    // given: nothing in flight, one packet in the queue
    let (_peer, addr) = peer_socket();
    let mut inner = wired(addr);
    queue_n(&mut inner, 1, addr);
    let inner = shared(inner);
    let mut timers = LoopTimers::new();
    // when
    let flow = notify_step(&inner, &mut timers, false).await;
    // then: the packet went out at once, without a timer
    assert!(flow.is_continue());
    assert_eq!(inner.lock().unwrap().outgoing.len(), 1);
}

#[tokio::test(start_paused = true)]
async fn notify_in_flight_defers_to_pacing_timer() {
    // given: two packets in flight, a third waits in the queue
    let (_peer, addr) = peer_socket();
    let mut inner = wired(addr);
    queue_n(&mut inner, 2, addr);
    let inner = shared(inner);
    let mut timers = LoopTimers::new();
    let _ = send_step(&inner, &mut timers, false).await;
    let third = vec![(2, build_data_packet(7, 2, 0, b"q"))];
    inner.lock().unwrap().queue_for_send(third, addr);
    // when
    let flow = notify_step(&inner, &mut timers, false).await;
    // then: the packet did not go out, the pacing timer is armed for "now"
    assert!(flow.is_continue());
    let guard = inner.lock().unwrap();
    assert_eq!(guard.outgoing.len(), 2);
    assert_eq!(guard.send_queue.len(), 1);
    assert_eq!(timers.pacing.deadline(), TokioInstant::now());
}

#[tokio::test(start_paused = true)]
async fn notify_on_shutdown_in_flight_sends_queue_and_breaks() {
    // given: two packets in flight, a third in the queue, shutdown in progress
    let (_peer, addr) = peer_socket();
    let mut inner = wired(addr);
    queue_n(&mut inner, 2, addr);
    let inner = shared(inner);
    let mut timers = LoopTimers::new();
    let _ = send_step(&inner, &mut timers, false).await;
    let third = vec![(2, build_data_packet(7, 2, 0, b"q"))];
    inner.lock().unwrap().queue_for_send(third, addr);
    // when
    let flow = notify_step(&inner, &mut timers, true).await;
    // then: the step ran at once, the whole queue went out, the loop ends
    assert!(flow.is_break());
    assert_eq!(inner.lock().unwrap().send_queue.len(), 0);
}

#[tokio::test(start_paused = true)]
async fn kick_by_timer_arms_pacing_for_now() {
    // given: fresh timers, pacing far in the future
    let mut timers = LoopTimers::new();
    // when
    kick_by_timer(&mut timers);
    // then
    assert_eq!(timers.pacing.deadline(), TokioInstant::now());
}

// ── choice of kick kind after an ACK ────────────────────────────────────────

#[test]
fn ack_kick_none_without_kick() {
    assert_eq!(ack_kick(false, false), AckKick::None);
    assert_eq!(ack_kick(false, true), AckKick::None);
}

#[test]
fn ack_kick_timer_without_ackstep() {
    assert_eq!(ack_kick(true, false), AckKick::Timer);
}

#[test]
fn ack_kick_inline_with_ackstep() {
    assert_eq!(ack_kick(true, true), AckKick::Inline);
}
