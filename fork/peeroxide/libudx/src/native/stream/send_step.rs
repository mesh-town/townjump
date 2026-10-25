// JS: libudx/src/udx.c (send_packets, pacing_timer_timeout, _udx_stream_write)
//! Sender step of the stream loop: batch to the socket and re-arm of the RTO, ZWP and pacing timers; step kicks (write — `notify_step`, ACK — `kick_by_timer`).

use super::timers::single_rto;
use super::*;
use std::ops::ControlFlow;
use std::pin::Pin;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::time::{Instant as TokioInstant, Sleep};

/// How many notifications one step drains; the rest goes to the next `select!` turn.
const DRAIN_MAX: usize = 64;

/// Four stream-loop timers; by default all far in the future.
pub(super) struct LoopTimers {
    pub(super) rto: Pin<Box<Sleep>>,
    pub(super) pacing: Pin<Box<Sleep>>,
    pub(super) zwp: Pin<Box<Sleep>>,
    pub(super) reo: Pin<Box<Sleep>>,
    /// Z: the step timer was armed via `kick_by_timer`, not by `after_step` itself; written only under `UDX_DIAG=z`.
    pub(super) by_kick: bool,
    /// Z: time of the first kick not yet served by a step; written only under `UDX_DIAG=z`.
    pub(super) kick_at: Option<std::time::Instant>,
}

impl LoopTimers {
    pub(super) fn new() -> Self {
        let far = || Box::pin(tokio::time::sleep(FAR_FUTURE));
        Self {
            rto: far(),
            pacing: far(),
            zwp: far(),
            reo: far(),
            by_kick: false,
            kick_at: None,
        }
    }

    pub(super) fn arm_rto(&mut self, after: Duration) {
        self.rto.as_mut().reset(TokioInstant::now() + after);
    }

    pub(super) fn arm_zwp(&mut self, after: Duration) {
        self.zwp.as_mut().reset(TokioInstant::now() + after);
    }

    pub(super) fn arm_reo(&mut self, after: Duration) {
        self.reo.as_mut().reset(TokioInstant::now() + after);
    }

    pub(super) fn arm_pacing(&mut self, after: Duration) {
        self.pacing.as_mut().reset(TokioInstant::now() + after);
    }
}

/// Drains notifications that have already arrived (no more than `DRAIN_MAX`); `true` if one of them was `Shutdown`.
pub(super) fn drain_notifies(
    first: StreamNotify,
    rx: &mut mpsc::UnboundedReceiver<StreamNotify>,
) -> bool {
    let rest = (0..DRAIN_MAX).map_while(|_| rx.try_recv().ok());
    std::iter::once(first)
        .chain(rest)
        .fold(false, |seen, n| seen | matches!(n, StreamNotify::Shutdown))
}

/// Result of lock 1: what to send and which timers to arm.
struct StepBatch {
    packets: Vec<(Vec<u8>, SocketAddr)>,
    udp: Option<Arc<UdpSocket>>,
    pacing_delay: Option<Duration>,
    arm_rto: Option<Duration>,
    zwp_start: Option<Duration>,
}

fn take_step(inner: &Arc<Mutex<StreamInner>>) -> StepBatch {
    let mut guard = inner.lock().unwrap_or_else(|e| e.into_inner());
    let was_outgoing_empty = guard.outgoing.is_empty();
    let batch = guard.take_send_batch();
    if diag::mode().z {
        guard.diag_step(batch.packets.len(), batch.rtx, batch.pacing_delay);
    }
    let should_arm = was_outgoing_empty && !guard.outgoing.is_empty();
    StepBatch {
        packets: batch.packets,
        udp: guard.udp.clone(),
        pacing_delay: batch.pacing_delay,
        arm_rto: should_arm.then(|| single_rto(guard.rto)),
        zwp_start: guard.zwp_start_on_write(),
    }
}

async fn send_all(udp: Option<Arc<UdpSocket>>, packets: &[(Vec<u8>, SocketAddr)]) {
    let Some(udp) = udp else { return };
    for (pkt, addr) in packets {
        let _sent = crate::native::tx_err::send_counted(&udp, pkt, *addr, "stream").await;
    }
}

/// Whether anything is left to send (queue or retransmissions; lock 2: a write could refill the queue during `send_to`).
fn queue_state(inner: &Arc<Mutex<StreamInner>>) -> bool {
    let guard = inner.lock().unwrap_or_else(|e| e.into_inner());
    guard.has_pending_send()
}

/// Pacing re-arm choice; `Break` — stop with the queue empty.
/// As C `udx.c:746-748`: the timer only when the pacing allowance is exhausted;
/// a closed window is woken by an ACK (`kick_pacing`).
pub(super) fn after_step(
    pacing_delay: Option<Duration>,
    has_more: bool,
    shutting_down: bool,
) -> ControlFlow<(), Duration> {
    if shutting_down && !has_more {
        return ControlFlow::Break(());
    }
    ControlFlow::Continue(pacing_delay.unwrap_or(FAR_FUTURE))
}

/// One step: batch to the socket, RTO/ZWP by the outcome, pacing re-arm.
pub(super) async fn send_step(
    inner: &Arc<Mutex<StreamInner>>,
    timers: &mut LoopTimers,
    shutting_down: bool,
) -> ControlFlow<()> {
    let step = take_step(inner);
    send_all(step.udp, &step.packets).await;
    step.arm_rto.into_iter().for_each(|d| timers.arm_rto(d));
    step.zwp_start.into_iter().for_each(|d| timers.arm_zwp(d));
    let has_more = queue_state(inner);
    match after_step(step.pacing_delay, has_more, shutting_down) {
        ControlFlow::Break(()) => ControlFlow::Break(()),
        ControlFlow::Continue(d) => {
            if diag::mode().z {
                timers.by_kick = false;
                timers.kick_at = None;
                if d == FAR_FUTURE {
                    diag::far_armed();
                }
            }
            timers.arm_pacing(d);
            ControlFlow::Continue(())
        }
    }
}

/// Kick via the pacing timer: the step goes at its nearest firing. By default an ACK is handled inline (`ackstep`),
/// the timer is used under `UDX_FIX=legacy`; a write on a busy stream goes through the timer the same way.
/// Stepping on every ACK or write during a bulk transfer causes many more kernel socket drops;
/// the timer caps the step rate. The inline step with the pacing bucket is the default (absent in legacy).
pub(super) fn kick_by_timer(timers: &mut LoopTimers) {
    if diag::mode().z {
        timers.by_kick = true;
        timers.kick_at.get_or_insert_with(std::time::Instant::now);
    }
    timers.arm_pacing(Duration::ZERO);
}

/// How to serve a kick after an ACK packet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AckKick {
    /// No kick.
    None,
    /// Via the pacing timer (`kick_by_timer`).
    Timer,
    /// Send step at once (default; under `UDX_FIX=legacy` via the timer).
    Inline,
}

/// Choice of kick kind by the ACK packet processing outcome.
pub(super) fn ack_kick(kick_pacing: bool, ackstep: bool) -> AckKick {
    match (kick_pacing, ackstep) {
        (false, _) => AckKick::None,
        (true, false) => AckKick::Timer,
        (true, true) => AckKick::Inline,
    }
}

/// Nothing in flight: all sent packets are acked.
fn nothing_in_flight(inner: &Arc<Mutex<StreamInner>>) -> bool {
    let guard = inner.lock().unwrap_or_else(|e| e.into_inner());
    guard.outgoing.is_empty()
}

/// Write or stop. From idle (nothing in flight) and on stop the step goes at once, without the timer: at window 1
/// the tokio timer rounds the firing up to a millisecond, which would dominate the per-datagram time.
/// While packets are in flight — via the timer, like an ACK (`kick_by_timer`).
pub(super) async fn notify_step(
    inner: &Arc<Mutex<StreamInner>>,
    timers: &mut LoopTimers,
    shutting_down: bool,
) -> ControlFlow<()> {
    if shutting_down || nothing_in_flight(inner) {
        return send_step(inner, timers, shutting_down).await;
    }
    kick_by_timer(timers);
    ControlFlow::Continue(())
}

#[cfg(test)]
#[path = "send_step_tests.rs"]
mod tests;
