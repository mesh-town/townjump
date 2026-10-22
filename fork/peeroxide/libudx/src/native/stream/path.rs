// JS: libudx/src/udx.c (udx_stream_send_message, process_packet MESSAGE, udx_stream_change_remote)
//! Path of a connected stream: MESSAGE packets, the wait for the first packet from the current remote, the path hook
//! that vets packets arriving from an address other than `remote_addr`; the public side is `control.rs`.

use super::wire::{message_packet, send_raw};
use super::*;

/// Decides whether a packet from a foreign source is processed; runs on the processor task without locks.
pub(super) type PathHook = Arc<dyn Fn(SocketAddr) -> bool + Send + Sync>;

/// The gate before the relay fast path (`true` - process the packet). The hook is cloned under the lock and runs
/// without it, so it may call `StreamControl` methods.
pub(super) fn path_gate(inner: &Arc<Mutex<StreamInner>>, addr: SocketAddr) -> bool {
    let hook = {
        let guard = inner.lock().unwrap_or_else(|e| e.into_inner());
        if !guard.connected || guard.remote_addr.is_none_or(|remote| remote == addr) {
            return true;
        }
        let Some(hook) = guard.path_hook.clone() else {
            return true;
        };
        hook
    };
    if hook(addr) {
        return true;
    }
    tracing::debug!(?addr, "path hook: dropping a packet from a foreign source");
    false
}

/// After a packet from `addr`: if it carries DATA or MESSAGE and `addr` is the current remote, wakes every `await_direct`.
pub(super) fn note_direct(inner: &mut StreamInner, header: &Header, addr: SocketAddr) {
    if header.type_flags & (FLAG_DATA | FLAG_MESSAGE) == 0 || inner.remote_addr != Some(addr) {
        return;
    }
    for waiter in inner.direct_waiters.drain(..) {
        let _ = waiter.send(());
    }
}

impl StreamInner {
    /// MESSAGE to the current remote in the background: not queued, no sequence number consumed. `false` - nothing to send to.
    pub(super) fn send_message(&self, payload: &[u8]) -> bool {
        let reachable = self.connected && self.udp.is_some() && self.remote_addr.is_some();
        if *self.closed.borrow() || !reachable {
            return false;
        }
        let seq = self.next_send_seq();
        send_raw(
            self,
            message_packet(self.remote_id, seq, self.next_remote_seq, payload),
        );
        true
    }

    /// Registers a waiter; on a closed stream the sender is dropped at once, so the receiver fails.
    pub(super) fn add_direct_waiter(&mut self) -> oneshot::Receiver<()> {
        let (tx, rx) = oneshot::channel();
        if !*self.closed.borrow() {
            self.direct_waiters.push(tx);
        }
        rx
    }
}
