// JS: libudx/src/udx.c
//! Public handle `UdxStream`: creation, connect, write/read/shutdown, relay, Drop; related — `firewall.rs`, `destroy.rs`.

use super::*;

// ── UdxStream ────────────────────────────────────────────────────────────────

/// A reliable, ordered, bidirectional byte stream over UDP.
///
/// Implements the UDX protocol with BBR congestion control, SACK-based
/// recovery, and RTT-driven retransmission. Multiple streams can share
/// a single [`super::socket::UdxSocket`] via stream ID multiplexing.
pub struct UdxStream {
    pub(super) local_id: u32,
    pub(crate) inner: Arc<Mutex<StreamInner>>,
    pub(super) processor: Mutex<Option<tokio::task::JoinHandle<()>>>,
    pub(super) incoming_tx: Option<mpsc::UnboundedSender<IncomingPacket>>,
    pub(super) incoming_rx: Mutex<Option<mpsc::UnboundedReceiver<IncomingPacket>>>,
    pub(crate) read_rx: Option<mpsc::UnboundedReceiver<StreamEvent>>,
    #[allow(dead_code)]
    close_rx: Option<oneshot::Receiver<()>>,
    pub(super) socket_streams: Mutex<Option<StreamMap>>,
}

impl UdxStream {
    pub(crate) fn new(local_id: u32) -> Self {
        let (read_tx, read_rx) = mpsc::unbounded_channel();
        let (close_tx, close_rx) = oneshot::channel();
        let (incoming_tx, incoming_rx) = mpsc::unbounded_channel();

        let inner = StreamInner::new(read_tx, Some(close_tx));

        Self {
            local_id,
            inner: Arc::new(Mutex::new(inner)),
            processor: Mutex::new(None),
            incoming_tx: Some(incoming_tx),
            incoming_rx: Mutex::new(Some(incoming_rx)),
            read_rx: Some(read_rx),
            close_rx: Some(close_rx),
            socket_streams: Mutex::new(None),
        }
    }

    /// Consume this stream and return a tokio-compatible async I/O adapter.
    pub fn into_async_stream(mut self) -> super::async_stream::UdxAsyncStream {
        let processor = self
            .processor
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        // Drop must not remove the entry while the task is alive (the task guard removes it); without
        // a processor there is no guard, and the entry is removed by the stream's Drop at the end of the method.
        if processor.is_some() {
            self.socket_streams
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take();
        }
        let Some(read_rx) = self.read_rx.take() else {
            return super::async_stream::UdxAsyncStream::new(
                Arc::clone(&self.inner),
                mpsc::unbounded_channel().1,
                None,
            );
        };
        let inner = Arc::clone(&self.inner);
        super::async_stream::UdxAsyncStream::new(inner, read_rx, processor)
    }

    /// Current effective MTU in bytes: the IP packet size, not UDP data.
    pub fn effective_mtu(&self) -> usize {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.mtu
    }

    /// Stream identifier in the socket table (the destination of the peer's packets).
    pub fn local_id(&self) -> u32 {
        self.local_id
    }

    /// Put the stream into the `socket` table without connecting: packets for `local_id` pile up in the stream's
    /// channel and are handled after `connect` on the same socket (`udx.c:2366` — the stream is visible to
    /// the socket from creation). `Drop` removes the entry.
    ///
    /// Before `connect` the channel is unbounded and not read: call only while waiting for a reply.
    /// Errors: socket not bound / stream already handed out / registered on another socket;
    /// `connect` on another socket after `register` returns `Err`.
    pub fn register(&self, socket: &super::socket::UdxSocket) -> Result<()> {
        socket.udp_arc()?;
        self.registered_elsewhere(socket)?;
        self.attach(socket)?;
        tracing::debug!(local_id = self.local_id, "stream registered");
        Ok(())
    }

    /// Clone of the sender into the `socket` table and a mark of where the stream is recorded (for `Drop`).
    pub(super) fn attach(&self, socket: &super::socket::UdxSocket) -> Result<()> {
        let tx = self.incoming_tx.as_ref();
        let tx =
            tx.ok_or_else(|| UdxError::Io(std::io::Error::other("stream already consumed")))?;
        socket.register_stream(self.local_id, tx.clone())?;
        let mut owner = self
            .socket_streams
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *owner = Some(socket.streams_ref());
        Ok(())
    }

    /// The stream is already in another socket's table (`register`, `set_firewall_hook`, `connect`):
    /// it cannot be connected on this one.
    fn registered_elsewhere(&self, socket: &super::socket::UdxSocket) -> Result<()> {
        let own = self
            .socket_streams
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let streams = socket.streams_ref();
        if own.as_ref().is_none_or(|m| Arc::ptr_eq(m, &streams)) {
            return Ok(());
        }
        Err(std::io::Error::other("stream registered on another socket").into())
    }

    /// Connect to a remote stream identified by `remote_id` at `remote_addr`.
    pub async fn connect(
        &self,
        socket: &super::socket::UdxSocket,
        remote_id: u32,
        remote_addr: SocketAddr,
    ) -> Result<()> {
        let udp = socket.udp_arc()?;
        if self.hook_on_other_socket(socket) {
            return self.connect_moving(socket, remote_id, remote_addr, udp);
        }
        self.registered_elsewhere(socket)?;
        let (notify_tx, notify_rx) = mpsc::unbounded_channel();

        {
            let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            inner.remote_id = remote_id;
            inner.remote_addr = Some(remote_addr);
            inner.apply_link_mtu(super::link_mtu::link_mtu(remote_addr));
            inner.udp = Some(udp);
            inner.connected = true;
            inner.notify_tx = Some(notify_tx);
        }

        self.attach(socket)?;

        self.spawn_processor(socket, notify_rx)?;

        tracing::debug!(
            local_id = self.local_id,
            remote_id,
            ?remote_addr,
            "stream connected"
        );
        Ok(())
    }

    /// Write `data` reliably. Resolves when the remote peer ACKs all packets.
    pub async fn write(&self, data: &[u8]) -> Result<()> {
        let prep = {
            let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            inner.prepare_write(data.len())?
        };

        send_data_packets(
            &self.inner,
            &prep.udp,
            prep.remote_addr,
            prep.remote_id,
            prep.first_seq,
            prep.current_ack,
            data,
        )
        .await?;

        prep.ack_rx.await.map_err(|_| UdxError::StreamClosed)?
    }

    /// Read the next message. Returns `None` on EOF or stream close.
    pub async fn read(&mut self) -> Result<Option<Vec<u8>>> {
        let rx = self.read_rx.as_mut().ok_or(UdxError::StreamClosed)?;
        match rx.recv().await {
            Some(StreamEvent::Data(d)) => Ok(Some(d)),
            Some(StreamEvent::End) => {
                self.read_rx = None;
                Ok(None)
            }
            None => {
                self.read_rx = None;
                Ok(None)
            }
        }
    }

    /// Send a write-end (FIN) and wait for the remote peer to ACK it.
    pub async fn shutdown(&self) -> Result<()> {
        let prep = {
            let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            inner.prepare_end()?
        };

        let header = Header {
            type_flags: FLAG_END,
            data_offset: 0,
            remote_id: prep.remote_id,
            recv_window: DEFAULT_RWND,
            seq: prep.first_seq,
            ack: prep.current_ack,
        };
        let packet = header.encode();

        {
            let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            guard.queue_for_send(vec![(prep.first_seq, packet.to_vec())], prep.remote_addr);
        }

        prep.ack_rx.await.map_err(|_| UdxError::StreamClosed)?
    }

    /// Set up packet-level relay forwarding to `destination`.
    ///
    /// Both streams must be on the same runtime. Relayed packets bypass
    /// congestion control — only the `remote_id` header field is rewritten.
    pub fn relay_to(&self, destination: &UdxStream) -> Result<()> {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.relay_target = Some(Arc::downgrade(&destination.inner));
        tracing::debug!("relay_to configured");
        Ok(())
    }
}

impl Drop for UdxStream {
    fn drop(&mut self) {
        let not_started = self.incoming_rx.lock().is_ok_and(|g| g.is_some());
        if not_started {
            // no processor, so no task guard: closing is up to the handle
            self.inner
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .mark_closed();
        }
        if let Ok(mut guard) = self.processor.lock() {
            if let Some(handle) = guard.take() {
                handle.abort();
            }
        }
        if let Ok(mut guard) = self.socket_streams.lock() {
            if let Some(streams) = guard.take() {
                if let Ok(mut map) = streams.lock() {
                    map.remove(&self.local_id);
                }
            }
        }
    }
}
