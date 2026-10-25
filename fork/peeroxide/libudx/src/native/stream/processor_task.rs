// JS: libudx/src/udx.c
//! Processor task start and the stream's entry guard in the socket table; see also `handle.rs`, `firewall.rs`.

use super::*;

/// Removes the stream's entry in the socket table when the processor task ends in any way:
/// normally, by panic or by `abort()` (the future is destroyed together with the guard).
/// `UdxStream::Drop` and `destroy` remove the entry themselves: there is no task before `connect`; removing twice
/// is harmless. Ids on one socket can repeat (relay_service and swarm have their own counters), and
/// removal after `abort()` is asynchronous, so we remove only our own entry: the same channel as ours.
pub(super) struct TableEntry {
    pub(super) streams: StreamMap,
    pub(super) local_id: u32,
    /// Weak: a strong clone would keep the channel open and the processor would not exit on `recv() == None`.
    pub(super) tx: mpsc::WeakUnboundedSender<IncomingPacket>,
}

impl Drop for TableEntry {
    fn drop(&mut self) {
        let mut map = self.streams.lock().unwrap_or_else(|e| e.into_inner());
        let ours = match (map.get(&self.local_id), self.tx.upgrade()) {
            (Some(live), Some(mine)) => live.same_channel(&mine),
            _ => false,
        };
        if ours {
            map.remove(&self.local_id);
        }
    }
}

/// Marks the stream closed when the processor task ends in any way (normally, by panic or by `abort()`).
struct ClosedGuard(Arc<Mutex<StreamInner>>);

impl Drop for ClosedGuard {
    fn drop(&mut self) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .mark_closed();
    }
}

impl UdxStream {
    /// Takes the stream's input channel and starts the processor; after the task starts, the table entry
    /// `socket` does not outlive it.
    pub(super) fn spawn_processor(
        &self,
        socket: &super::socket::UdxSocket,
        notify_rx: mpsc::UnboundedReceiver<StreamNotify>,
    ) -> Result<()> {
        let incoming_rx = self
            .incoming_rx
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .ok_or_else(|| UdxError::Io(std::io::Error::other("processor already started")))?;
        let tx = self.incoming_tx.as_ref();
        let tx =
            tx.ok_or_else(|| UdxError::Io(std::io::Error::other("stream already consumed")))?;
        let inner = Arc::clone(&self.inner);
        let closed = ClosedGuard(Arc::clone(&inner));
        let entry = TableEntry {
            streams: socket.streams_ref(),
            local_id: self.local_id,
            tx: tx.downgrade(),
        };
        let handle = tokio::spawn(async move {
            let _closed = closed;
            let _entry = entry;
            process_incoming(inner, incoming_rx, notify_rx).await;
        });
        *self.processor.lock().unwrap_or_else(|e| e.into_inner()) = Some(handle);
        Ok(())
    }
}
