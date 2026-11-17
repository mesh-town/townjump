//! Local abort of a stream through `StreamControl::abort`: the processor sends DESTROY and ends.
// JS: udx-native/lib/stream.js (destroy)

use super::*;

impl StreamInner {
    /// Asks the processor to end the stream; without a live processor the stream is terminated here.
    pub(super) fn request_abort(&mut self) {
        if *self.closed.borrow() {
            return;
        }
        self.abort_requested = true;
        let woken = self
            .notify_tx
            .as_ref()
            .filter(|_| self.connected)
            .is_some_and(|tx| tx.send(StreamNotify::DataQueued).is_ok());
        if !woken {
            self.terminate();
        }
    }
}

/// Run by the processor after a notification: if an abort was requested, tell the peer (DESTROY), end the reader
/// with a reset and report `true` so the loop ends the stream like it does on a received DESTROY.
pub(super) async fn take_abort(inner: &Arc<Mutex<StreamInner>>) -> bool {
    let destroy = {
        let mut guard = inner.lock().unwrap_or_else(|e| e.into_inner());
        if !guard.abort_requested {
            return false;
        }
        let destroy = guard.destroy_packet();
        guard.connected = false;
        let _ = guard.read_tx.send(StreamEvent::End);
        if let Some(tx) = guard.close_tx.take() {
            let _ = tx.send(());
        }
        destroy
    };
    if let Some((udp, addr, bytes)) = destroy {
        let _sent = crate::native::tx_err::send_counted(&udp, &bytes, addr, "abort").await;
    }
    true
}

#[cfg(test)]
#[path = "abort_tests.rs"]
mod tests;
