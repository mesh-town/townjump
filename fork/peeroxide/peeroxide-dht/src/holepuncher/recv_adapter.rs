//! Receive adapter of a [`Holepuncher`]: turns holepunch probes arriving on a
//! socket into `Connected` events naming the receiving socket (initiator) or reflections (passive side).

use super::HolepunchEvent;
use crate::socket_pool::{HolepunchEvent as ProbeEvent, SocketRef};
use libudx::UdxSocket;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// Take ownership of `socket_ref`'s holepunch-probe receiver and spawn a
/// [`run_recv_adapter`] task for it, sharing the same `connected_flag` and
/// `event_tx` as every other socket belonging to the same [`Holepuncher`].
/// Every socket that can receive a `PEER_HOLEPUNCH` probe — the primary
/// socket acquired in `Holepuncher::new`, and each birthday-attack socket
/// opened by `open_birthday_sockets` — must be covered this way, or a probe
/// landing on an uncovered socket will silently never fire `Connected`.
///
/// Returns `None` (spawning nothing) if `socket_ref`'s `holepunch_rx` has
/// already been taken; this should not happen for a freshly acquired
/// `SocketRef` but is handled defensively rather than panicking.
pub(super) fn spawn_recv_adapter_for_socket(
    socket_ref: &mut SocketRef,
    is_initiator: bool,
    event_tx: mpsc::UnboundedSender<HolepunchEvent>,
    connected_flag: Arc<AtomicBool>,
) -> Option<JoinHandle<()>> {
    let hp_rx = socket_ref.take_holepunch_rx()?;
    let socket_clone = socket_ref.socket.clone();
    Some(tokio::spawn(run_recv_adapter(
        hp_rx,
        socket_clone,
        is_initiator,
        event_tx,
        connected_flag,
    )))
}

async fn run_recv_adapter(
    mut hp_rx: mpsc::UnboundedReceiver<ProbeEvent>,
    socket: UdxSocket,
    is_initiator: bool,
    event_tx: mpsc::UnboundedSender<HolepunchEvent>,
    connected_flag: Arc<AtomicBool>,
) {
    while let Some(probe) = hp_rx.recv().await {
        if is_initiator {
            if connected_flag
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                tracing::info!(
                    target: "peeroxide::_events::holepunch::probe_received",
                    addr = %probe.addr,
                    "holepuncher(initiator): probe received, firing Connected"
                );
                let _ = event_tx.send(HolepunchEvent::Connected {
                    addr: probe.addr,
                    socket: socket.clone(),
                });
                return;
            }
        } else {
            tracing::info!(
                target: "peeroxide::_events::holepunch::passive_reflected",
                addr = %probe.addr,
                "holepuncher(passive): probe received, reflecting"
            );
            let _ = socket.send_to(&[0u8], probe.addr);
        }
    }
}
