// JS: hyperdht/lib/server.js
//! One PEER_HANDSHAKE, from the client's first Noise message to our reply
//! (JS `_onpeerhandshake` and `_addHandshake`, lines 200-470).

use std::time::Instant;

use libudx::UdxSocket;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::oneshot;

use super::actor::{Internal, State};
use super::conn::{Pending, arm_hook, remote_udx_id};
use super::opts::{FirewallInput, rejects};
use super::plan::{
    Plan, client_agrees, client_lan, dial_from_arrival, plan, reply_from_puncher, same_host,
};
use super::punch::PunchState;
use super::relay;
use super::reply::{Ours, encode_reply, handshake_payload, reply_error};
use super::reply_build::{Prepared, dial_client, ours, prepare, refuse};
use super::table::{Begin, INITIAL_TIMEOUT};
use crate::handshake_route::HandshakeRoute;
use crate::hyperdht::{HyperDhtError, alloc_stream_id};
use crate::hyperdht_messages::{
    ERROR_NONE, FIREWALL_UNKNOWN, HandshakeMessage, NoisePayload, RelayThroughInfo,
};
use crate::messages::Ipv4Peer;
use crate::noise_wrap::NoiseWrap;
use crate::peer::NodeId;

/// The fields of `ServerEvent::PeerHandshake` that the server needs.
pub(super) struct Hello {
    pub(super) msg: HandshakeMessage,
    pub(super) from: Ipv4Peer,
    pub(super) peer_address: Option<Ipv4Peer>,
    pub(super) target: Option<NodeId>,
    pub(super) route: HandshakeRoute,
}

/// A client that passed the firewall: the Noise session after `recv`.
struct Admitted {
    nw: NoiseWrap,
    remote: NoisePayload,
    client_address: Ipv4Peer,
    direct: bool,
    arrived_on: UdxSocket,
    /// The relay we announce in every reply to this handshake (chosen once).
    relay_through: Option<RelayThroughInfo>,
}

/// What happens after the reply went out.
pub(super) enum Next {
    /// Nothing: refused, or an error reply.
    Done,
    /// The server dials the client at this address.
    Dial(Pending, Ipv4Peer),
    /// The connection waits for the client's first packet (or the punch).
    Wait(Pending),
}

pub(super) struct Built {
    pub(super) noise_reply: Vec<u8>,
    pub(super) next: Next,
    /// The hole-punch socket when the relayed answer must leave from it.
    pub(super) via: Option<UdxSocket>,
}

/// Answers one handshake: `reply_tx` gets the wire bytes, or `None` for silence.
pub(super) async fn on_handshake(
    st: &mut State,
    hello: Hello,
    reply_tx: oneshot::Sender<Option<Vec<u8>>>,
    internal: &UnboundedSender<Internal>,
) {
    let now = Instant::now();
    let id = match st.table.begin(&hello.msg.noise, now) {
        Begin::Repeat(stored) => {
            if let Some(via) = st.table.via_of(&hello.msg.noise) {
                hello.route.reply_via.set(via);
            }
            let _ = reply_tx.send(stored.and_then(|noise| encode_reply(&hello.msg, noise)));
            return;
        }
        Begin::Fresh(id) => id,
    };
    let built = attempt(st, id, &hello, internal).await;
    let noise_reply = built.as_ref().map(|b| b.noise_reply.clone());
    st.table.finish(id, noise_reply.clone());
    remember_via(st, id, &hello, built.as_ref());
    let _ = reply_tx.send(noise_reply.and_then(|noise| encode_reply(&hello.msg, noise)));
    if let Some(built) = built {
        after_reply(st, id, built.next, now, internal);
    }
}

/// Tells the router (and later repeats of this Noise message) to answer from the punch socket.
fn remember_via(st: &mut State, id: u64, hello: &Hello, built: Option<&Built>) {
    let Some(via) = built.and_then(|b| b.via.clone()) else {
        return;
    };
    hello.route.reply_via.set(via.clone());
    st.table.set_via(id, via);
}

async fn attempt(
    st: &State,
    id: u64,
    hello: &Hello,
    internal: &UnboundedSender<Internal>,
) -> Option<Built> {
    let admitted = admit(st, hello)?;
    respond(st, id, admitted, internal).await
}

/// Steps 2-4: are we listening for this target, does the Noise message
/// parse, does the firewall let the client in.
fn admit(st: &State, hello: &Hello) -> Option<Admitted> {
    let listening = st.listening.as_ref().filter(|_| !st.suspended)?;
    if hello.target != Some(listening.target) {
        return None;
    }
    let mut nw = NoiseWrap::new_responder(listening.key_pair.to_noise_keypair());
    let remote = nw.recv(&hello.msg.noise);
    let remote = remote.inspect_err(|e| tracing::debug!(err = %e, "server: noise recv failed"));
    let remote = remote.ok()?;
    let remote_public_key = nw.remote_public_key()?;
    let client_address = hello
        .peer_address
        .clone()
        .unwrap_or_else(|| hello.from.clone());
    let input = FirewallInput {
        remote_public_key: &remote_public_key,
        remote_payload: &remote,
        client_address: &client_address,
    };
    if rejects(&st.opts, &input) {
        tracing::debug!(client = %client_address.host, "server: firewall refused the client");
        return None;
    }
    let direct = hello.peer_address.is_none();
    Some(Admitted {
        nw,
        remote,
        client_address,
        direct,
        arrived_on: hello.route.arrived_on.clone(),
        relay_through: relay::own_relay(&st.opts),
    })
}

/// Steps 5-9: the plan, the sockets, the stream, the Noise reply.
async fn respond(
    st: &State,
    id: u64,
    adm: Admitted,
    internal: &UnboundedSender<Internal>,
) -> Option<Built> {
    let error = reply_error(&adm.remote);
    let firewall = st.dht.firewalled().await.unwrap_or(FIREWALL_UNKNOWN);
    if error != ERROR_NONE {
        return refuse(adm.nw, ours(st, id, error, firewall, adm.relay_through));
    }
    let our = st.dht.remote_address().await.ok().flatten();
    let same = same_host(
        our.as_ref(),
        &adm.client_address,
        &adm.remote.addresses4,
        adm.direct,
    );
    let plan = plan(adm.remote.firewall, adm.direct, firewall);
    tracing::debug!(
        plan = ?plan,
        same_host = same,
        our_host = our.as_ref().map_or("-", |o| o.host.as_str()),
        client_host = %adm.client_address.host,
        client_agrees = client_agrees(&adm.client_address, &adm.remote.addresses4),
        client_lan = client_lan(&adm.client_address, &adm.remote.addresses4),
        remote_fw = adm.remote.firewall,
        direct = adm.direct,
        "server: plan"
    );
    let remote_fw = adm.remote.firewall;
    let arrival = dial_from_arrival(plan, adm.direct).then(|| adm.arrived_on.clone());
    let mut prep = prepare(st, plan, remote_fw, arrival)
        .await
        .inspect_err(|e| tracing::debug!(err = %e, "server: sockets not ready"))
        .ok()?;
    match wire(st, id, adm, plan, &mut prep, firewall, internal).await {
        Ok(built) => Some(built),
        Err(e) => {
            tracing::debug!(err = %e, "server: handshake reply failed");
            prep.punch.take().into_iter().for_each(PunchState::destroy);
            None
        }
    }
}

/// Steps 8-9: the stream, its firewall hook, the Noise reply and the final session.
async fn wire(
    st: &State,
    id: u64,
    adm: Admitted,
    plan: Plan,
    prep: &mut Prepared,
    firewall: u64,
    internal: &UnboundedSender<Internal>,
) -> Result<Built, HyperDhtError> {
    let Admitted {
        mut nw,
        remote,
        client_address,
        relay_through,
        ..
    } = adm;
    let remote_udx_id = remote_udx_id(&remote)?;
    let relay = relay::attempt(plan, relay_through.as_ref(), remote.relay_through.as_ref());
    let local_id = alloc_stream_id();
    let stream = st.rt.create_stream(local_id).await?;
    if plan != Plan::DialClient {
        arm_hook(&stream, &prep.socket, remote_udx_id, id, internal)?;
    }
    let addresses4 = prep.addresses4.clone();
    let udx_id = u64::from(local_id);
    let payload = Ours {
        addresses4,
        udx_id,
        ..ours(st, id, ERROR_NONE, firewall, relay_through)
    };
    let noise_reply = nw.send(&handshake_payload(&payload))?;
    let noise = nw.finalize()?;
    if let Some(punch) = prep.punch.as_mut() {
        punch.arm(noise.holepunch_secret);
    }
    let socket = prep.socket.clone();
    let reusable = remote.udx.as_ref().is_some_and(|u| u.reusable_socket);
    let keep = (st.opts.reusable_socket && reusable).then(|| st.dht.reuse.clone());
    let pending = Pending {
        stream,
        socket,
        noise,
        remote_udx_id,
        punch: prep.punch.take(),
        keep,
        relay,
    };
    let via = reply_from_puncher(plan).then(|| pending.socket.clone());
    Ok(Built {
        noise_reply,
        next: next(plan, pending, client_address),
        via,
    })
}

fn next(plan: Plan, pending: Pending, client_address: Ipv4Peer) -> Next {
    match plan {
        Plan::DialClient => Next::Dial(pending, client_address),
        Plan::AwaitClient | Plan::Holepunch => Next::Wait(pending),
    }
}

/// Step 11: dial now, or park the connection until the client arrives.
fn after_reply(
    st: &mut State,
    id: u64,
    next: Next,
    now: Instant,
    internal: &UnboundedSender<Internal>,
) {
    match next {
        Next::Done => st.table.clear_later(id, now),
        Next::Wait(mut pending) => {
            let local_id = pending.stream.local_id();
            let deadline = pending.relay.as_mut().map(|r| {
                relay::start(st, r, id, local_id, internal);
                r.deadline
            });
            let deadline = deadline.unwrap_or(now + INITIAL_TIMEOUT);
            st.table.attach(id, pending, deadline);
        }
        Next::Dial(pending, client) => {
            st.table.clear_later(id, now);
            dial_client(st, pending, &client);
        }
    }
}

#[cfg(test)]
#[path = "handshake_tests.rs"]
mod tests;
