// JS: hyperdht/lib/server.js
//! Builders of the server's reply pieces: error replies, our side of the
//! Noise payload, and the dial back to the client.

use libudx::UdxSocket;

use super::actor::State;
use super::conn::{self, Pending, to_socket_addr};
use super::handshake::{Built, Next};
use super::plan::Plan;
use super::punch::{self, PunchState};
use super::reply::{Ours, handshake_payload};
use crate::holepuncher::Holepuncher;
use crate::hyperdht::HyperDhtError;
use crate::hyperdht_messages::RelayThroughInfo;
use crate::messages::Ipv4Peer;
use crate::noise_wrap::NoiseWrap;

/// Error reply (version mismatch, no UDX info): no stream, the client gives up.
pub(super) fn refuse(mut nw: NoiseWrap, ours: Ours) -> Option<Built> {
    let noise_reply = nw.send(&handshake_payload(&ours)).ok()?;
    nw.finalize().ok()?;
    Some(Built {
        noise_reply,
        next: Next::Done,
        via: None,
    })
}

pub(super) fn ours(
    st: &State,
    id: u64,
    error: u64,
    firewall: u64,
    relay_through: Option<RelayThroughInfo>,
) -> Ours {
    Ours {
        error,
        firewall,
        addresses4: vec![],
        holepunch_id: id,
        relays: st.relays(),
        udx_id: 0,
        reusable_socket: st.opts.reusable_socket,
        relay_through,
    }
}

pub(super) fn dial_client(st: &State, pending: Pending, client: &Ipv4Peer) {
    match to_socket_addr(client) {
        Some(addr) => {
            tokio::spawn(conn::dial(pending, addr, st.incoming.clone()));
        }
        None => pending.destroy(),
    }
}

/// Sockets and addresses prepared before the reply is built.
pub(super) struct Prepared {
    pub(super) socket: UdxSocket,
    pub(super) addresses4: Vec<Ipv4Peer>,
    pub(super) punch: Option<PunchState>,
}

/// Step 7: the puzzle socket when we must punch, the socket a direct request came in on
/// (`arrival`), the DHT primary socket otherwise.
pub(super) async fn prepare(
    st: &State,
    plan: Plan,
    remote_fw: u64,
    arrival: Option<UdxSocket>,
) -> Result<Prepared, HyperDhtError> {
    if plan == Plan::Holepunch {
        let (punch, socket, port) = punch::setup(&st.dht, &st.rt, remote_fw).await?;
        let addresses4 = Holepuncher::handshake_addresses4(&punch.punch_addrs, port);
        return Ok(Prepared {
            socket,
            addresses4,
            punch: Some(punch),
        });
    }
    let socket = match arrival {
        Some(socket) => socket,
        None => st
            .dht
            .server_socket()
            .await?
            .ok_or(HyperDhtError::ChannelClosed)?,
    };
    let port = socket.local_addr().await?.port();
    let addresses4 = st.dht.noise_addresses4(port).await;
    Ok(Prepared {
        socket,
        addresses4,
        punch: None,
    })
}
