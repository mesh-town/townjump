// JS: libudx/src/udx.c (udx_stream_change_remote, reset_mtu_state_machine)
//! Route change of a connected stream: the state of a pending change, its completion by ACK, stream closing, and the
//! `connect` that moves a stream with a pending firewall hook to another socket; the public side is `control.rs`.

use super::*;
use crate::native::socket::UdxSocket;

/// A pending route change: it completes when the peer ACKs everything sent before it (`seq` and below).
pub(super) struct RemoteChange {
    pub(super) seq: u32,
    pub(super) done: Option<oneshot::Sender<()>>,
}

/// The stub address of a packet queued before the firewall hook fired.
pub(super) fn stub_addr() -> SocketAddr {
    SocketAddr::from(([0, 0, 0, 0], 0))
}

/// Adds the stream's entry to `socket`'s table. `None` — our channel is already there; a live foreign channel under the id is an error.
pub(super) fn table_entry(
    socket: &UdxSocket,
    local_id: u32,
    tx: &mpsc::UnboundedSender<IncomingPacket>,
) -> Result<Option<TableEntry>> {
    let streams = socket.streams_ref();
    let live = streams
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&local_id)
        .cloned();
    match live {
        Some(live) if live.same_channel(tx) => return Ok(None),
        Some(live) if !live.is_closed() => {
            return Err(std::io::Error::other("stream id taken on target socket").into());
        }
        _ => {}
    }
    socket.register_stream(local_id, tx.clone())?;
    Ok(Some(TableEntry {
        streams,
        local_id,
        tx: tx.downgrade(),
    }))
}

impl StreamInner {
    /// The checks of `change_remote`; the state stays untouched on an error.
    pub(super) fn check_changeable(&self) -> Result<()> {
        if *self.closed.borrow() {
            return Err(UdxError::StreamClosed);
        }
        if !self.connected {
            return Err(std::io::Error::other("Not yet connected").into());
        }
        if self.change.is_some() {
            return Err(std::io::Error::other("Remote already changing").into());
        }
        Ok(())
    }

    /// Applies the new route under the caller's lock; `None` — applied at once, `Some` — completes on ACK of `next_seq`.
    /// Packets already sent keep their old address (`outgoing.stamp_route`); the queue keeps its own addresses.
    pub(super) fn begin_change(
        &mut self,
        udp: Arc<tokio::net::UdpSocket>,
        remote_id: u32,
        addr: SocketAddr,
    ) -> Option<oneshot::Receiver<()>> {
        if let Some(old) = self.remote_addr {
            self.outgoing.stamp_route(old);
        }
        self.remote_id = remote_id;
        self.remote_addr = Some(addr);
        self.udp = Some(udp);
        self.reset_mtu(addr);
        let seq = self.next_seq;
        if seq == self.remote_acked {
            return None;
        }
        let (done, rx) = oneshot::channel();
        self.change = Some(RemoteChange {
            seq,
            done: Some(done),
        });
        Some(rx)
    }

    /// Back to the base MTU and the ladder of a new stream (udx.c `reset_mtu_state_machine`), then the link limit of `addr`.
    fn reset_mtu(&mut self, addr: SocketAddr) {
        self.mtu_state = MtuState::Base;
        self.mtu = MTU_BASE;
        self.mtu_probe_size = MTU_BASE + MTU_STEP;
        self.mtu_probe_count = 0;
        self.mtu_probe_wanted = false;
        self.mtu_max = MTU_MAX;
        self.apply_link_mtu(super::link_mtu::link_mtu(addr));
    }

    /// From `on_ack_packet`: the ACK reached the change point.
    pub(super) fn settle_change(&mut self) {
        let Some(change) = self.change.as_mut() else {
            return;
        };
        if seq_diff(self.remote_acked, change.seq) < 0 {
            return;
        }
        if let Some(done) = change.done.take() {
            let _ = done.send(());
        }
        self.change = None;
    }

    /// A packet sent to `addr` other than the current remote (the queue outlived a route change) keeps that route.
    pub(super) fn note_route(&mut self, seq: u32, addr: SocketAddr) {
        if Some(addr) == self.remote_addr || addr == stub_addr() {
            return;
        }
        self.outgoing.set_route(Seq(seq), Some(addr));
    }

    /// Idempotent: the stream is closed for good; a waiting change fails with `StreamClosed`, extra table entries go.
    pub(super) fn mark_closed(&mut self) {
        self.closed.send_replace(true);
        self.change = None;
        self.direct_waiters.clear();
        self.extra_tables.clear();
    }
}

impl UdxStream {
    /// A firewall hook is armed on another socket's table: `connect` here moves the stream instead of failing.
    pub(super) fn hook_on_other_socket(&self, socket: &UdxSocket) -> bool {
        let own = self
            .socket_streams
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let Some(own) = own.as_ref() else {
            return false;
        };
        let armed = self
            .inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .firewall_hook
            .is_some();
        armed && !Arc::ptr_eq(own, &socket.streams_ref())
    }

    /// `connect` for a stream with an armed hook on another socket: the hook goes, the old entry stays, the processor is the same.
    pub(super) fn connect_moving(
        &self,
        socket: &UdxSocket,
        remote_id: u32,
        remote_addr: SocketAddr,
        udp: Arc<tokio::net::UdpSocket>,
    ) -> Result<()> {
        let tx = self.incoming_tx.as_ref();
        let tx =
            tx.ok_or_else(|| UdxError::Io(std::io::Error::other("stream already consumed")))?;
        let entry = table_entry(socket, self.local_id, tx)?;
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if inner.firewall_hook.take().is_none() {
            return Err(std::io::Error::other("stream registered on another socket").into());
        }
        inner.firewall_socket = None;
        inner.remote_id = remote_id;
        inner.remote_addr = Some(remote_addr);
        inner.apply_link_mtu(super::link_mtu::link_mtu(remote_addr));
        inner.udp = Some(udp);
        inner.connected = true;
        inner.send_queue.readdress(stub_addr(), remote_addr);
        inner.extra_tables.extend(entry);
        if let Some(notify) = inner.notify_tx.as_ref() {
            let _ = notify.send(StreamNotify::DataQueued);
        }
        tracing::debug!(
            local_id = self.local_id,
            remote_id,
            ?remote_addr,
            "stream moved to another socket"
        );
        Ok(())
    }
}
