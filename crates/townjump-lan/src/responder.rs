//! Responder: answers LAN requests for the targets it serves.

use std::collections::{BTreeSet, HashMap, HashSet};
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
use crate::interfaces::InterfaceSource;
use crate::membership;

#[derive(Default)]
struct State {
    targets: Mutex<HashMap<Target, u16>>,
    paused: AtomicBool,
}

/// Listens on the multicast group and answers requests for registered targets.
pub struct Responder {
    state: Arc<State>,
    task: JoinHandle<()>,
    socket: Arc<UdpSocket>,
    group: Ipv4Addr,
    source: InterfaceSource,
    joined: Mutex<HashSet<Ipv4Addr>>,
}

/// Binds the group port (shared with other responders on the host); the group is joined by `membership`.
fn bind_socket(cfg: &LanConfig) -> io::Result<UdpSocket> {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    socket.set_reuse_address(true)?;
    #[cfg(unix)]
    socket.set_reuse_port(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, cfg.port).into())?;
    UdpSocket::from_std(socket.into())
}

/// A bound socket joined to the group on every interface of the host (for fake peers in tests).
#[cfg(test)]
pub(crate) fn bind_group_socket(cfg: &LanConfig) -> io::Result<UdpSocket> {
    let socket = bind_socket(cfg)?;
    let current = InterfaceSource::system().list();
    membership::sync(&socket, cfg.group, &mut HashSet::new(), &current);
    Ok(socket)
}

impl Responder {
    /// Binds the socket and starts answering. Must run inside a tokio runtime.
    pub fn start(cfg: &LanConfig) -> io::Result<Self> {
        Self::start_with(cfg, InterfaceSource::system())
    }

    /// Like `start`, with the interfaces taken from `source`.
    pub fn start_with(cfg: &LanConfig, source: InterfaceSource) -> io::Result<Self> {
        let socket = Arc::new(bind_socket(cfg)?);
        let state = Arc::new(State::default());
        let task = tokio::spawn(run(socket.clone(), state.clone()));
        let joined = Mutex::new(HashSet::new());
        let group = cfg.group;
        let responder = Self {
            state,
            task,
            socket,
            group,
            source,
            joined,
        };
        responder.refresh_interfaces();
        Ok(responder)
    }

    /// Walks the interfaces again: joins the group on new ones, forgets vanished ones.
    pub fn refresh_interfaces(&self) {
        let current = self.source.list();
        let mut joined = self.joined.lock().unwrap();
        membership::sync(&self.socket, self.group, &mut joined, &current);
    }

    /// Interfaces the group is joined on, ascending.
    pub fn joined_interfaces(&self) -> Vec<Ipv4Addr> {
        let joined = self.joined.lock().unwrap();
        joined
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
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

async fn run(socket: Arc<UdpSocket>, state: Arc<State>) {
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
