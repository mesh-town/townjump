//! Responder: answers LAN requests for the targets it serves.

use std::collections::HashMap;
use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::UdpSocket;
use tokio::task::JoinHandle;

use crate::LanConfig;
use crate::codec::{self, Reply, Target};
use crate::interfaces::ipv4_interfaces;

#[derive(Default)]
struct State {
    targets: Mutex<HashMap<Target, u16>>,
    paused: AtomicBool,
}

/// Listens on the multicast group and answers requests for registered targets.
pub struct Responder {
    state: Arc<State>,
    task: JoinHandle<()>,
}

/// Binds the group port (shared with other responders on the host) and joins
/// the group on every non-loopback IPv4 interface.
pub(crate) fn bind_group_socket(cfg: &LanConfig) -> io::Result<UdpSocket> {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    socket.set_reuse_address(true)?;
    #[cfg(unix)]
    socket.set_reuse_port(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, cfg.port).into())?;
    for iface in ipv4_interfaces() {
        if let Err(e) = socket.join_multicast_v4(&cfg.group, &iface) {
            tracing::debug!("lan: cannot join {} on {iface}: {e}", cfg.group);
        }
    }
    UdpSocket::from_std(socket.into())
}

impl Responder {
    /// Binds the socket and starts answering. Must run inside a tokio runtime.
    pub fn start(cfg: &LanConfig) -> io::Result<Self> {
        let socket = bind_group_socket(cfg)?;
        let state = Arc::new(State::default());
        let task = tokio::spawn(run(socket, state.clone()));
        Ok(Self { state, task })
    }

    /// Starts answering requests for `target` with `port`.
    pub fn register(&self, target: Target, port: u16) {
        self.state.targets.lock().unwrap().insert(target, port);
    }

    /// Stops answering requests for `target`.
    pub fn unregister(&self, target: &Target) {
        self.state.targets.lock().unwrap().remove(target);
    }

    /// While paused the responder stays silent (for suspend).
    pub fn set_paused(&self, paused: bool) {
        self.state.paused.store(paused, Ordering::SeqCst);
    }

    /// Stops the responder task and releases the socket.
    pub fn close(&self) {
        self.task.abort();
    }
}

impl Drop for Responder {
    fn drop(&mut self) {
        self.close();
    }
}

async fn run(socket: UdpSocket, state: Arc<State>) {
    let mut buf = [0u8; 256];
    loop {
        let (n, src) = match socket.recv_from(&mut buf).await {
            Ok(v) => v,
            Err(e) => {
                tracing::debug!("lan: receive failed: {e}");
                tokio::time::sleep(Duration::from_millis(10)).await;
                continue;
            }
        };
        if state.paused.load(Ordering::SeqCst) {
            continue;
        }
        let Some(req) = codec::decode_request(&buf[..n]) else {
            continue;
        };
        let port = state.targets.lock().unwrap().get(&req.target).copied();
        let (Some(port), SocketAddr::V4(_)) = (port, src) else {
            continue;
        };
        let reply = codec::encode_reply(&Reply {
            nonce: req.nonce,
            target: req.target,
            port,
        });
        if let Err(e) = socket.send_to(&reply, src).await {
            tracing::debug!("lan: cannot reply to {src}: {e}");
        }
    }
}
