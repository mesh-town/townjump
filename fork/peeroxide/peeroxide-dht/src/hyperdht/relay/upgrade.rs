//! The switch from the relay to the direct path: a detached supervisor per connection that
//! moves the stream, confirms the peer is reachable directly, then closes the relay.
// JS: hyperdht/lib/connect.js:793-838, hyperdht/lib/server.js (relay pair)

use std::net::SocketAddr;
use std::sync::Mutex;

use libudx::{StreamControl, UdxSocket};
use tokio::sync::{oneshot, watch};

use super::pair::RelaySession;
use super::{AbortOnDrop, RelayLink, RelayPath};
use crate::hyperdht::connection::ConnectResult;
use crate::hyperdht::types::HyperDhtError;

/// Where the direct path comes from.
pub(crate) enum DirectSource {
    /// Client: the hole punch gave the peer's address.
    #[allow(dead_code)]
    // the client builds its source in `spawn_after_punch`; tests use this one
    Punched {
        socket: UdxSocket,
        addr: SocketAddr,
        remote_id: u32,
    },
    /// Server: the peer's first packet from a new address names it.
    PathHook { socket: UdxSocket, remote_id: u32 },
}

/// The direct target, known now or after the peer's first foreign packet.
enum Target {
    Ready(SocketAddr),
    Wait(oneshot::Receiver<SocketAddr>),
}

type PunchTask = AbortOnDrop<Result<(ConnectResult, UdxSocket), HyperDhtError>>;

pub(crate) struct Upgrade;

impl Upgrade {
    /// Supervise `control` with a direct path source; the relay `session` stays open until the switch
    /// is confirmed or the stream closes. The task is detached: dropping the link keeps it.
    pub(crate) fn spawn(
        control: StreamControl,
        session: RelaySession,
        direct: DirectSource,
    ) -> RelayLink {
        let (tx, rx) = watch::channel(RelayPath::Relayed);
        let (socket, remote_id, target) = target_of(&control, direct);
        tokio::spawn(supervise(control, session, tx, socket, remote_id, target));
        RelayLink::from_path(rx)
    }

    /// As `spawn`, with the hole punch still running: its success is the direct source,
    /// its failure leaves the connection on the relay until the stream closes.
    pub(crate) fn spawn_after_punch(
        control: StreamControl,
        session: RelaySession,
        mut punch: PunchTask,
    ) -> RelayLink {
        let (tx, rx) = watch::channel(RelayPath::Relayed);
        tokio::spawn(async move {
            let closed = control.closed();
            let punched = tokio::select! {
                _ = closed => None,
                joined = &mut punch.0 => joined.ok().and_then(Result::ok),
            };
            tracing::debug!(target: "peeroxide_dht::hyperdht::relay", ok = punched.is_some(), "upgrade: punch finished");
            let Some((result, socket)) = punched else {
                return stay_relayed(control, session, tx).await;
            };
            let Some((remote_id, addr)) = remote_of(&result) else {
                return stay_relayed(control, session, tx).await;
            };
            supervise(control, session, tx, socket, remote_id, Target::Ready(addr)).await;
        });
        RelayLink::from_path(rx)
    }
}

fn remote_of(result: &ConnectResult) -> Option<(u32, SocketAddr)> {
    let id = u32::try_from(result.remote_udx.as_ref()?.id).ok()?;
    let host = result.server_address.host.parse().ok()?;
    Some((id, SocketAddr::new(host, result.server_address.port)))
}

fn target_of(control: &StreamControl, direct: DirectSource) -> (UdxSocket, u32, Target) {
    match direct {
        DirectSource::Punched {
            socket,
            addr,
            remote_id,
        } => (socket, remote_id, Target::Ready(addr)),
        DirectSource::PathHook { socket, remote_id } => {
            let (tx, rx) = oneshot::channel();
            let once = Mutex::new(Some(tx));
            let relay = control.remote().map(|(addr, _)| addr);
            // Always true: the leftovers the relay still delivers must reach the stream.
            control.set_path_hook(move |src| {
                if relay.is_some_and(|relay| is_relay_alias(relay, src)) {
                    return true;
                }
                tracing::debug!(target: "peeroxide_dht::hyperdht::relay", %src, "upgrade: first foreign packet");
                let sender = once.lock().ok().and_then(|mut g| g.take());
                if let Some(sender) = sender {
                    let _ = sender.send(src);
                }
                true
            });
            (socket, remote_id, Target::Wait(rx))
        }
    }
}

/// The relay answering from another address of the same host (a relay bound to all interfaces replies
/// from loopback to a peer it was reached by its LAN address): same port, loopback on either side.
fn is_relay_alias(relay: SocketAddr, src: SocketAddr) -> bool {
    if relay.port() != src.port() {
        return false;
    }
    relay.ip() == src.ip() || relay.ip().is_loopback() || src.ip().is_loopback()
}

/// No direct path: keep the relay until the stream closes.
async fn stay_relayed(
    control: StreamControl,
    session: RelaySession,
    _tx: watch::Sender<RelayPath>,
) {
    control.closed().await;
    session.close().await;
}

/// Every step races the stream closing; a confirmed switch closes the relay and flips the path.
async fn supervise(
    control: StreamControl,
    session: RelaySession,
    tx: watch::Sender<RelayPath>,
    socket: UdxSocket,
    remote_id: u32,
    target: Target,
) {
    let closed = control.closed();
    tokio::pin!(closed);
    let switched = tokio::select! {
        _ = &mut closed => return session.close().await,
        done = switch(&control, &socket, remote_id, target) => done,
    };
    if !switched {
        closed.await;
        return session.close().await;
    }
    session.close().await;
    tx.send_replace(RelayPath::Direct);
    // The direct socket's receive loop stops with its last handle: hold it while the stream is alive.
    closed.await;
    drop(socket);
}

/// Move the stream, tell the peer, wait for its first direct packet, confirm back. `false`: stay on the relay.
async fn switch(control: &StreamControl, socket: &UdxSocket, id: u32, target: Target) -> bool {
    let addr = match target {
        Target::Ready(addr) => addr,
        Target::Wait(rx) => match rx.await {
            Ok(addr) => addr,
            Err(_) => return false,
        },
    };
    tracing::debug!(target: "peeroxide_dht::hyperdht::relay", %addr, id, "upgrade: moving the stream to the direct path");
    // Armed before the change: the peer's direct message can beat our own confirmation.
    let direct = control.await_direct();
    let changed = match control.change_remote(socket, id, addr) {
        Ok(changed) => changed,
        Err(e) => {
            tracing::debug!(target: "peeroxide_dht::hyperdht::relay", "change_remote failed: {e}");
            return false;
        }
    };
    if changed.await.is_err() {
        tracing::debug!(target: "peeroxide_dht::hyperdht::relay", "upgrade: change_remote not confirmed");
        return false;
    }
    if !matches!(control.try_send_message(&[]), Ok(true)) {
        tracing::debug!(target: "peeroxide_dht::hyperdht::relay", "upgrade: MESSAGE not sent");
        return false;
    }
    tracing::debug!(target: "peeroxide_dht::hyperdht::relay", "upgrade: MESSAGE sent, waiting for the peer");
    let ok = direct.await.is_ok();
    // The peer waits for a packet that comes after its own switch: the packet that made us
    // switch (the server's trigger) does not count for it, so answer the proof with a confirmation.
    if ok {
        let _ = control.try_send_message(&[]);
    }
    tracing::debug!(target: "peeroxide_dht::hyperdht::relay", ok, "upgrade: direct path wait finished");
    ok
}

#[cfg(test)]
#[path = "upgrade_tests.rs"]
mod tests;
