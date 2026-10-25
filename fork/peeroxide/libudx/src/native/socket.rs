// JS: libudx/src/udx.c:udx_socket_bind (SO_SNDBUF, SO_RCVBUF ladder, reading the size back, "don't fragment"),
//     libudx/src/io_posix.c, io_win.c:udx__udp_set_dontfrag.
// Related files: socket_dontfrag.rs ("don't fragment"), socket_recv.rs (receive loop: what to skip), socket_buffers_tests.rs (buffer tests), stream.rs (streams over the socket).
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, OnceLock};

use socket2::SockRef;
use tokio::sync::mpsc;

use crate::error::{Result, UdxError};

/// The reference's `UDX_DEFAULT_SNDBUF_SIZE`: the SO_SNDBUF size requested right after bind.
const SNDBUF_SIZE: usize = 212_992;

/// The reference's SO_RCVBUF ladder. On macOS `setsockopt` fails if asked for more than the system will allocate,
/// so we step down to the first success; 208 KiB is "the old maximum, known to work".
const RCVBUF_LADDER: [usize; 4] = [1024 * 1024, 512 * 1024, 256 * 1024, 208 * 1024];

/// An incoming unreliable datagram received on a [`UdxSocket`].
#[derive(Debug, Clone)]
pub struct Datagram {
    /// Raw payload bytes.
    pub data: Vec<u8>,
    /// Source address of the datagram.
    pub addr: SocketAddr,
}

struct UdxSocketInner {
    udp: OnceLock<Arc<tokio::net::UdpSocket>>,
    recv_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    streams: Arc<Mutex<HashMap<u32, mpsc::UnboundedSender<super::stream::IncomingPacket>>>>,
    fallback_tx: Arc<Mutex<Option<mpsc::UnboundedSender<Datagram>>>>,
}

impl UdxSocketInner {
    fn new() -> Self {
        Self {
            udp: OnceLock::new(),
            recv_task: Mutex::new(None),
            streams: Arc::new(Mutex::new(HashMap::new())),
            fallback_tx: Arc::new(Mutex::new(None)),
        }
    }

    fn udp_arc(&self) -> Result<Arc<tokio::net::UdpSocket>> {
        Ok(Arc::clone(self.udp.get().ok_or_else(|| {
            UdxError::Io(std::io::Error::other("socket not bound"))
        })?))
    }

    fn ensure_recv_loop(&self) -> Result<()> {
        let mut guard = self.recv_task.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(ref handle) = *guard {
            if !handle.is_finished() {
                return Ok(());
            }
        }

        let udp = self.udp_arc()?;
        let streams = Arc::clone(&self.streams);
        let fallback_tx = Arc::clone(&self.fallback_tx);

        let handle = tokio::spawn(async move {
            let mut buf = vec![0u8; 65536];
            while let Some((len, addr)) = recv::recv_next(&udp, &mut buf).await {
                if super::stream::diag::mode().z {
                    super::stream::diag::count_recv();
                }
                let packet = buf[..len].to_vec();

                if len >= super::header::HEADER_SIZE {
                    if let Ok(hdr) = super::header::Header::decode(&packet) {
                        let guard = streams.lock().unwrap_or_else(|e| e.into_inner());
                        if let Some(tx) = guard.get(&hdr.remote_id) {
                            let _ = tx.send(super::stream::IncomingPacket {
                                data: packet,
                                addr,
                                stamp: super::stream::diag::mode().z.then(std::time::Instant::now),
                            });
                            continue;
                        }
                    }
                }

                let guard = fallback_tx.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(ref tx) = *guard {
                    let _ = tx.send(Datagram { data: packet, addr });
                }
            }
        });

        *guard = Some(handle);
        Ok(())
    }

    fn close_impl(&self) {
        if let Ok(mut guard) = self.recv_task.lock() {
            if let Some(handle) = guard.take() {
                handle.abort();
            }
        }
    }
}

impl Drop for UdxSocketInner {
    fn drop(&mut self) {
        self.close_impl();
    }
}

/// A UDP socket used for UDX stream transport and unreliable datagrams.
///
/// `UdxSocket` is a cheap-clone handle: cloning it increments a reference
/// count and all clones share the same underlying UDP socket. The socket is
/// closed (receive loop aborted) when the *last* clone is dropped.
///
/// Incoming packets are demultiplexed: UDX stream packets (identified by
/// header magic + stream ID) are routed to the owning [`super::stream::UdxStream`],
/// while non-UDX packets are delivered as [`Datagram`]s via [`recv_start`](Self::recv_start).
#[derive(Clone)]
pub struct UdxSocket {
    inner: Arc<UdxSocketInner>,
}

impl UdxSocket {
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(UdxSocketInner::new()),
        }
    }

    /// Whether both handles refer to the same socket (clones are equal, distinct sockets are not).
    pub fn ptr_eq(&self, other: &UdxSocket) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    /// Bind the socket to a local address. Returns an error if already bound.
    pub async fn bind(&self, addr: SocketAddr) -> Result<()> {
        let socket = tokio::net::UdpSocket::bind(addr).await?;
        configure_buffers(&socket)?;
        configure_dontfrag(&socket, addr.is_ipv6());
        self.inner
            .udp
            .set(Arc::new(socket))
            .map_err(|_| UdxError::Io(std::io::Error::other("socket already bound")))?;
        Ok(())
    }

    /// Return the local address this socket is bound to.
    pub async fn local_addr(&self) -> Result<SocketAddr> {
        let udp = self
            .inner
            .udp
            .get()
            .ok_or_else(|| UdxError::Io(std::io::Error::other("socket not bound")))?;
        Ok(udp.local_addr()?)
    }

    /// Set the IP time-to-live (IPv4 `IP_TTL` / IPv6 hop limit) used for
    /// outgoing datagrams sent on this socket.
    ///
    /// A low TTL is used by NAT-holepunching probes that intentionally
    /// expire before reaching the remote peer (birthday-attack probes that
    /// must not trigger a "connection refused" ICMP response visible to the
    /// remote NAT), while the default TTL is used for regular traffic.
    pub fn set_ttl(&self, ttl: u32) -> Result<()> {
        let udp = self.inner.udp_arc()?;
        Ok(udp.set_ttl(ttl)?)
    }

    /// Return the currently configured IP time-to-live for this socket.
    pub fn ttl(&self) -> Result<u32> {
        let udp = self.inner.udp_arc()?;
        Ok(udp.ttl()?)
    }

    /// Send one datagram with IP TTL `ttl` and put the socket TTL back right after
    /// (JS `udx_socket_send_ttl`). Synchronous; a full send buffer (`WouldBlock`) is returned as an error.
    /// Not re-entrant: the TTL is socket-wide for the call, so other sends in that window and a concurrent call on the same socket see `ttl` too.
    /// Not counted by `tx_err` (unlike `send_to`).
    pub fn send_to_ttl(&self, data: &[u8], addr: SocketAddr, ttl: u32) -> Result<()> {
        let udp = self.inner.udp_arc()?;
        let prev = udp.ttl()?;
        udp.set_ttl(ttl)?;
        // Plain syscall: tokio's try_send_to is WouldBlock before the reactor's first turn.
        let sent = SockRef::from(&*udp).send_to(data, &addr.into());
        udp.set_ttl(prev)?;
        sent?;
        Ok(())
    }

    /// Get a shared reference to the underlying UDP socket.
    pub(crate) fn udp_arc(&self) -> Result<Arc<tokio::net::UdpSocket>> {
        self.inner.udp_arc()
    }

    /// Register a stream to receive packets addressed to `local_id`.
    pub(crate) fn register_stream(
        &self,
        local_id: u32,
        tx: mpsc::UnboundedSender<super::stream::IncomingPacket>,
    ) -> Result<()> {
        self.inner
            .streams
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(local_id, tx);
        self.inner.ensure_recv_loop()?;
        Ok(())
    }

    pub(crate) fn streams_ref(
        &self,
    ) -> Arc<Mutex<HashMap<u32, mpsc::UnboundedSender<super::stream::IncomingPacket>>>> {
        Arc::clone(&self.inner.streams)
    }

    /// Send an unreliable datagram to `addr`. Fire-and-forget.
    pub fn send_to(&self, data: &[u8], addr: SocketAddr) -> Result<()> {
        let udp = self.inner.udp_arc()?;
        let data = data.to_vec();
        tokio::spawn(async move {
            let _sent = crate::native::tx_err::send_counted(&udp, &data, addr, "socket").await;
        });
        Ok(())
    }

    /// Begin receiving non-stream datagrams on this socket.
    pub fn recv_start(&self) -> Result<mpsc::UnboundedReceiver<Datagram>> {
        let (tx, rx) = mpsc::unbounded_channel();
        *self
            .inner
            .fallback_tx
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(tx);
        self.inner.ensure_recv_loop()?;
        Ok(rx)
    }

    /// Close the socket and stop the receive loop.
    ///
    /// Consumes `self`. Because `UdxSocket` is a clone handle, the socket is
    /// only actually closed when every clone has either been dropped or
    /// `close`d and the internal `Arc` reaches zero.
    pub async fn close(self) -> Result<()> {
        let inner = self.inner.clone();
        drop(self);
        if Arc::strong_count(&inner) == 0 {
            inner.close_impl();
        }
        Ok(())
    }
}

/// The first ladder size that `set` accepted; failure on the whole ladder is an error with the cause of the last failure.
fn set_first_accepted(
    ladder: &[usize],
    mut set: impl FnMut(usize) -> std::io::Result<()>,
) -> std::io::Result<usize> {
    let mut last_refusal = std::io::Error::other("empty ladder");
    for &size in ladder {
        match set(size) {
            Ok(()) => return Ok(size),
            Err(e) => last_refusal = e,
        }
    }
    Err(std::io::Error::new(
        last_refusal.kind(),
        format!("SO_RCVBUF: every size of {ladder:?} refused, last error: {last_refusal}"),
    ))
}

/// Buffer size in the same units as the request: Linux returns the kernel-doubled value, libuv (the reference) halves it.
fn as_requested(raw: usize) -> usize {
    match cfg!(any(target_os = "linux", target_os = "android")) {
        true => raw / 2,
        false => raw,
    }
}

/// Buffer size request right after bind, as in the reference; "less than requested" is a debug log entry only.
fn configure_buffers(udp: &tokio::net::UdpSocket) -> Result<()> {
    let sock = SockRef::from(udp);
    sock.set_send_buffer_size(SNDBUF_SIZE)?;
    let requested = set_first_accepted(&RCVBUF_LADDER, |n| sock.set_recv_buffer_size(n))?;
    let actual = as_requested(sock.recv_buffer_size()?);
    if actual < requested {
        tracing::debug!(requested, actual, "SO_RCVBUF: less than requested");
    }
    Ok(())
}

#[path = "socket_dontfrag.rs"]
mod dontfrag;
use dontfrag::configure_dontfrag;

#[path = "socket_recv.rs"]
mod recv;

#[cfg(test)]
#[path = "socket_buffers_tests.rs"]
mod buffer_tests;
