//! Shared UDP socket pool used by NAT hole-punching.
//!
//! `SocketPool` hands out `SocketRef` references to ephemeral UDP
//! sockets bound on local ports, multiplexing receive of incoming
//! `PEER_HOLEPUNCH` probes through a dedicated channel (`HolepunchEvent`).
//! The [`crate::holepuncher::Holepuncher`] uses this pool to launch the
//! birthday-attack probe sequence required to traverse symmetric NATs.
//!
//! Most consumers use the pool indirectly via
//! [`crate::hyperdht::HyperDhtHandle::connect`]; direct use is only needed
//! for custom DHT-server orchestration or low-level hole-punch experiments.

#![allow(missing_docs)]

use std::net::SocketAddr;

use libudx::{Datagram, UdxRuntime, UdxSocket};
use tokio::sync::{mpsc, oneshot};

use crate::messages::{Command, Message};
use crate::rpc::DhtHandle;

#[path = "socket_pool_requests.rs"]
mod requests;
use requests::spawn_request_forwarder;
pub(crate) use requests::{DHT_REQUEST_QUEUE, DhtRequestSeen};

const HOLEPUNCH_MSG: &[u8] = &[0];
const HOLEPUNCH_TTL: u32 = 5;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SocketPoolError {
    #[error("udx error: {0}")]
    Udx(#[from] libudx::UdxError),
    #[error("invalid address: {0}")]
    AddrParse(#[from] std::net::AddrParseError),
}

pub type Result<T> = std::result::Result<T, SocketPoolError>;

/// A UDX frame that reached a pooled socket: the socket (a clone) and the sender's address.
pub(crate) struct UdxFrameSeen {
    pub(crate) socket: UdxSocket,
    pub(crate) addr: SocketAddr,
}

type FrameSink = (UdxSocket, mpsc::UnboundedSender<UdxFrameSeen>);
type RequestLane = (UdxSocket, mpsc::Sender<DhtRequestSeen>);

pub struct SocketPool {
    host: String,
    udx_frames: Option<mpsc::UnboundedSender<UdxFrameSeen>>,
}

impl SocketPool {
    pub fn new(host: String) -> Self {
        Self {
            host,
            udx_frames: None,
        }
    }

    /// Reports every UDX frame that reaches a socket of this pool (frames that no stream of the
    /// socket claims) instead of dropping it.
    pub(crate) fn with_udx_frames(mut self, tx: mpsc::UnboundedSender<UdxFrameSeen>) -> Self {
        self.udx_frames = Some(tx);
        self
    }

    pub async fn acquire(&self, runtime: &UdxRuntime) -> Result<SocketRef> {
        let socket = runtime.create_socket().await?;
        let addr: SocketAddr = format!("{}:0", self.host).parse()?;
        socket.bind(addr).await?;

        let (hp_tx, hp_rx) = mpsc::unbounded_channel();
        let (dht_tx, dht_rx) = mpsc::unbounded_channel();
        let recv_rx = socket.recv_start()?;

        let frames = self
            .udx_frames
            .as_ref()
            .map(|tx| (socket.clone(), tx.clone()));
        let (req_tx, req_rx) = mpsc::channel(DHT_REQUEST_QUEUE);
        let (owner_tx, owner_rx) = oneshot::channel();
        let lane = Some((socket.clone(), req_tx));
        let recv_task = tokio::spawn(route_messages(
            recv_rx, hp_tx, dht_tx, frames, lane, owner_rx,
        ));

        Ok(SocketRef {
            socket,
            holepunch_rx: Some(hp_rx),
            dht_reply_rx: Some(dht_rx),
            dht_request_rx: Some(req_rx),
            _owner: owner_tx,
            _recv_task: Some(recv_task),
        })
    }
}

/// Resolves when the receiving end of the frame sink is gone; never without a sink.
async fn sink_closed(frames: &Option<FrameSink>) {
    match frames {
        Some((_, tx)) => tx.closed().await,
        None => std::future::pending().await,
    }
}

/// Resolves when the owning `SocketRef` is gone; never without a lane.
async fn owner_gone(lane: &Option<RequestLane>, owner: &mut oneshot::Receiver<()>) {
    if lane.is_none() {
        return std::future::pending().await;
    }
    let _ = owner.await;
}

async fn route_messages(
    mut recv_rx: mpsc::UnboundedReceiver<Datagram>,
    hp_tx: mpsc::UnboundedSender<HolepunchEvent>,
    dht_tx: mpsc::UnboundedSender<Datagram>,
    mut frames: Option<FrameSink>,
    mut lane: Option<RequestLane>,
    mut owner: oneshot::Receiver<()>,
) {
    loop {
        let dgram = tokio::select! {
            d = recv_rx.recv() => match d {
                Some(d) => d,
                None => break,
            },
            // The sink holds a clone of the socket; once nobody listens, let go of it so the
            // socket can close.
            _ = sink_closed(&frames) => {
                frames = None;
                continue;
            }
            // The lane holds a clone of the socket too; release it with the owner.
            _ = owner_gone(&lane, &mut owner) => {
                lane = None;
                continue;
            }
        };
        match classify_inbound(&dgram.data) {
            InboundClass::Holepunch => {
                tracing::debug!(addr = %dgram.addr, len = dgram.data.len(), "socket_pool: holepunch probe received");
                let _ = hp_tx.send(HolepunchEvent { addr: dgram.addr });
            }
            InboundClass::DhtResponse => {
                tracing::trace!(addr = %dgram.addr, len = dgram.data.len(), "socket_pool: DHT response forwarded");
                let _ = dht_tx.send(dgram);
            }
            InboundClass::UdxFrame => {
                // Frames of known streams never get here; this is a frame for a stream that is
                // not on this socket. Report it when a sink is set, else drop it.
                if let Some((socket, tx)) = &frames {
                    let seen = UdxFrameSeen {
                        socket: socket.clone(),
                        addr: dgram.addr,
                    };
                    if tx.send(seen).is_err() {
                        frames = None;
                    }
                }
            }
            InboundClass::DhtRequest => match &lane {
                Some((socket, tx)) => {
                    let seen = DhtRequestSeen {
                        addr: dgram.addr,
                        data: dgram.data,
                        via: socket.clone(),
                    };
                    let addr = seen.addr;
                    match tx.try_send(seen) {
                        Ok(()) => {
                            tracing::debug!(%addr, "socket_pool: DHT ping queued for the DHT actor")
                        }
                        Err(_) => {
                            tracing::trace!(%addr, "socket_pool: DHT ping dropped, queue full or closed")
                        }
                    }
                }
                None => {
                    tracing::trace!(addr = %dgram.addr, "socket_pool: DHT ping dropped, no owner")
                }
            },
            InboundClass::Drop => {
                tracing::trace!(addr = %dgram.addr, len = dgram.data.len(), "socket_pool: non-ping datagram dropped");
            }
        }
    }
}

#[derive(Debug)]
#[non_exhaustive]
pub struct HolepunchEvent {
    pub addr: SocketAddr,
}

pub struct SocketRef {
    pub socket: UdxSocket,
    pub holepunch_rx: Option<mpsc::UnboundedReceiver<HolepunchEvent>>,
    pub dht_reply_rx: Option<mpsc::UnboundedReceiver<Datagram>>,
    /// Internal PING requests queued until [`Self::serve_dht_requests`] attaches the actor.
    pub(crate) dht_request_rx: Option<mpsc::Receiver<DhtRequestSeen>>,
    _owner: oneshot::Sender<()>,
    _recv_task: Option<tokio::task::JoinHandle<()>>,
}

impl SocketRef {
    /// The holepunch receiver is `Some(...)` immediately after `SocketPool::acquire()`.
    /// The Holepuncher recv-adapter takes ownership through this accessor exactly once;
    /// subsequent calls return `None`.
    pub fn take_holepunch_rx(&mut self) -> Option<mpsc::UnboundedReceiver<HolepunchEvent>> {
        self.holepunch_rx.take()
    }

    /// Take ownership of the DHT-reply receiver. Datagrams whose payload
    /// `messages::decode_message` parses as a `Message::Response` are
    /// forwarded here. Used by `Nat::auto_sample` to receive PING-via-socket
    /// replies through the puncher socket's own NAT mapping.
    pub fn take_dht_reply_rx(&mut self) -> Option<mpsc::UnboundedReceiver<Datagram>> {
        self.dht_reply_rx.take()
    }

    /// Starts answering internal PING requests that reach this socket through `dht`. A second
    /// call does nothing.
    pub(crate) fn serve_dht_requests(&mut self, dht: &DhtHandle) {
        if let Some(rx) = self.dht_request_rx.take() {
            spawn_request_forwarder(dht.clone(), rx);
        }
    }

    pub fn send_holepunch(&self, addr: SocketAddr, low_ttl: bool) -> Result<()> {
        if low_ttl {
            return Ok(self
                .socket
                .send_to_ttl(HOLEPUNCH_MSG, addr, HOLEPUNCH_TTL)?);
        }
        Ok(self.socket.send_to(HOLEPUNCH_MSG, addr)?)
    }

    pub fn send_holepunch_to(&self, host: &str, port: u16, low_ttl: bool) -> Result<()> {
        let addr: SocketAddr = format!("{host}:{port}").parse()?;
        self.send_holepunch(addr, low_ttl)
    }
}

pub fn random_port() -> u16 {
    1000 + (rand::random::<f64>() * 64536.0) as u16
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum InboundClass {
    Holepunch,
    UdxFrame,
    DhtResponse,
    DhtRequest,
    Drop,
}

/// Classify an inbound datagram on a holepunch socket. UDX frames are
/// libudx's responsibility (demuxed before they reach this lane) so they
/// are dropped defensively; `decode_message` parses the rest. Responses go
/// to the reply lane, internal PING requests to the request lane, and
/// everything else is dropped.
pub(crate) fn classify_inbound(buf: &[u8]) -> InboundClass {
    if buf.len() <= 1 {
        return InboundClass::Holepunch;
    }
    if buf.len() >= 20 && buf[0] == 0xFF {
        return InboundClass::UdxFrame;
    }
    match crate::messages::decode_message(buf) {
        Ok(Message::Response(_)) => InboundClass::DhtResponse,
        Ok(Message::Request(r)) if r.internal && r.command == Command::Ping as u64 => {
            InboundClass::DhtRequest
        }
        _ => InboundClass::Drop,
    }
}

/// Map a raw `firewall` value into the punch-strategy bucket the Holepuncher
/// dispatcher understands.
///
/// Matches Node `hyperdht/lib/holepuncher.js:333-335`: only `OPEN →
/// CONSISTENT`. `UNKNOWN` no longer gets silently coerced — by the time
/// `punch()` runs the puncher's `Nat` has been seeded by
/// [`crate::holepuncher::Holepuncher::auto_sample`] which settles the
/// firewall to `CONSISTENT` / `RANDOM` via real reflexive samples.
pub fn coerce_firewall(fw: u64) -> u64 {
    use crate::hyperdht_messages::{FIREWALL_CONSISTENT, FIREWALL_OPEN};
    if fw == FIREWALL_OPEN {
        FIREWALL_CONSISTENT
    } else {
        fw
    }
}

#[cfg(test)]
#[path = "socket_pool_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "socket_pool_request_tests.rs"]
mod request_tests;
