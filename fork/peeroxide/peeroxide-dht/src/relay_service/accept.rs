// JS: hyperdht/lib/server.js
//! Control connections of the relay service: they arrive from a regular
//! [`crate::server::Server`] (so a client behind a NAT is hole-punched or dialed back), and
//! each one runs a relay session on its own UDX socket.

use tokio::sync::mpsc;

use crate::hyperdht::{HolepunchFilter, PeerConnection};
use crate::hyperdht_messages::FIREWALL_RANDOM;
use crate::protomux::Mux;
use crate::server::ServerOpts;

use super::session::run_relay_session;
use super::{RelayServiceConfig, Service};

/// Options of the control-connection server: the admission callback, a reusable socket and
/// the holepunch veto for random-NAT peers.
pub(super) fn server_opts(config: &RelayServiceConfig) -> ServerOpts {
    ServerOpts {
        firewall: config.peer_firewall.clone(),
        reusable_socket: true,
        holepunch: Some(relay_holepunch_filter()),
        ..ServerOpts::default()
    }
}

/// Refuses to punch exactly when our own NAT is random (no port can be predicted).
pub(super) fn relay_holepunch_filter() -> HolepunchFilter {
    HolepunchFilter::new(|input| input.local_firewall != FIREWALL_RANDOM)
}

/// Start a session task for every incoming control connection until the channel closes.
pub(super) async fn accept_loop(svc: Service, mut incoming: mpsc::Receiver<PeerConnection>) {
    while let Some(conn) = incoming.recv().await {
        svc.tasks.spawn(run_session(svc.clone(), conn));
    }
}

/// Drive the relay session of one control connection; the rest of the connection (socket
/// lease, relay link) lives until the session ends.
async fn run_session(svc: Service, conn: PeerConnection) {
    let socket = conn.socket.clone();
    let remote_public_key = conn.remote_public_key;
    let (mux, run) = Mux::new(conn.stream);
    svc.tasks.spawn(run);
    run_relay_session(
        svc.relay,
        mux,
        remote_public_key,
        svc.matched_tx,
        svc.sockets,
        socket,
    )
    .await;
}

#[cfg(test)]
#[path = "accept_tests.rs"]
mod accept_tests;

#[cfg(test)]
#[path = "admit_tests.rs"]
mod admit_tests;
