//! Per-handshake path of the relay service: admit or silently refuse one
//! `PEER_HANDSHAKE`, answer it and finalize the encrypted control connection.
//! Siblings: `session`, `bridge`, `stop`.
// JS: blind-relay/index.js

use std::sync::Arc;
use std::time::Instant;

use libudx::{RuntimeHandle, UdxRuntime};

use crate::hyperdht::{HyperDhtError, HyperDhtHandle, KeyPair, alloc_stream_id};
use crate::hyperdht_messages::{
    HandshakeMessage, MODE_FROM_RELAY, MODE_FROM_SECOND_RELAY, MODE_FROM_SERVER, MODE_REPLY,
    NoisePayload, SecretStreamInfo, UdxInfo, encode_handshake_to_bytes,
};
use crate::messages::Ipv4Peer;
use crate::noise::Keypair as NoiseKeypair;
use crate::noise_wrap::{NoiseWrap, NoiseWrapResult};
use crate::protomux::Mux;
use crate::secret_stream::SecretStream;
use crate::server::{Firewall, FirewallInput, rejects_with};

use super::Service;
use super::dedup::Dedup;
use super::session::run_relay_session;
use super::stop::TaskSet;

/// What decides the answer to a handshake: our identity, the advertised NAT
/// state and the admission callback (`None` admits all).
pub(super) struct Gate<'a> {
    pub(super) key_pair: &'a KeyPair,
    pub(super) firewall: u64,
    pub(super) peer_firewall: Option<&'a Firewall>,
}

/// A control connection to finalize after the reply went out.
pub(super) struct NewSession {
    pub(super) local_stream_id: u32,
    pub(super) remote_udx: UdxInfo,
    pub(super) noise: NoiseWrapResult,
}

/// What to do with one `PEER_HANDSHAKE`: the encoded reply (`None` = stay
/// silent) and the session to start, if any. A repeat starts no session.
pub(super) struct Response {
    pub(super) reply: Option<Vec<u8>>,
    pub(super) session: Option<NewSession>,
}

/// Decide the answer to one handshake; a repeated Noise message gets the
/// stored reply (silence for a refused peer) and no second session.
pub(super) fn respond(
    dedup: &mut Dedup,
    gate: &Gate<'_>,
    msg: &HandshakeMessage,
    from: &Ipv4Peer,
    peer_address: Option<Ipv4Peer>,
    addresses4: Vec<Ipv4Peer>,
    now: Instant,
) -> Response {
    if let Some(reply) = dedup.known(&msg.noise, now) {
        return Response {
            reply,
            session: None,
        };
    }
    let response = answer(gate, msg, from, peer_address, addresses4).unwrap_or(Response {
        reply: None,
        session: None,
    });
    dedup.remember(msg.noise.clone(), response.reply.clone(), now);
    response
}

fn answer(
    gate: &Gate<'_>,
    msg: &HandshakeMessage,
    from: &Ipv4Peer,
    peer_address: Option<Ipv4Peer>,
    addresses4: Vec<Ipv4Peer>,
) -> Option<Response> {
    let noise_kp = NoiseKeypair {
        public_key: gate.key_pair.public_key,
        secret_key: gate.key_pair.secret_key,
    };
    let mut nw = NoiseWrap::new_responder(noise_kp);
    let remote_payload = nw
        .recv(&msg.noise)
        .inspect_err(|e| tracing::debug!(err = %e, "relay handshake: noise recv failed"))
        .ok()?;
    let remote_public_key = nw.remote_public_key()?;
    let client_address = peer_address.as_ref().unwrap_or(from);
    if refused(gate, &remote_public_key, &remote_payload, client_address) {
        tracing::debug!(client = %client_address.host, "relay: firewall refused the peer");
        return None;
    }
    if remote_payload.error != 0 {
        tracing::debug!(
            error = remote_payload.error,
            "relay handshake: remote reported error"
        );
        return None;
    }

    let local_stream_id = alloc_stream_id();
    let noise_reply = nw
        .send(&reply_payload(gate.firewall, addresses4, local_stream_id))
        .inspect_err(|e| tracing::debug!(err = %e, "relay handshake: noise send failed"))
        .ok()?;
    let noise = nw
        .finalize()
        .inspect_err(|e| tracing::debug!(err = %e, "relay handshake: noise finalize failed"))
        .ok()?;

    let (reply_mode, reply_peer_address) = match msg.mode {
        MODE_FROM_RELAY | MODE_FROM_SECOND_RELAY => (MODE_FROM_SERVER, peer_address),
        _ => (MODE_REPLY, None),
    };
    let reply_msg = HandshakeMessage {
        mode: reply_mode,
        noise: noise_reply,
        peer_address: reply_peer_address,
        relay_address: None,
    };
    let reply = encode_handshake_to_bytes(&reply_msg).ok();
    let session = remote_payload.udx.map(|remote_udx| NewSession {
        local_stream_id,
        remote_udx,
        noise,
    });
    if session.is_none() {
        tracing::debug!("relay: connecting peer advertised no UDX info");
    }
    Some(Response { reply, session })
}

/// The admission callback refuses this peer (a panic refuses too).
fn refused(
    gate: &Gate<'_>,
    remote_public_key: &[u8; 32],
    remote_payload: &NoisePayload,
    client_address: &Ipv4Peer,
) -> bool {
    let input = FirewallInput {
        remote_public_key,
        remote_payload,
        client_address,
    };
    rejects_with(gate.peer_firewall, &input)
}

fn reply_payload(firewall: u64, addresses4: Vec<Ipv4Peer>, local_stream_id: u32) -> NoisePayload {
    NoisePayload {
        version: 1,
        error: 0,
        firewall,
        holepunch: None,
        addresses4,
        addresses6: vec![],
        udx: Some(UdxInfo {
            version: 1,
            reusable_socket: true,
            id: u64::from(local_stream_id),
            seq: 0,
        }),
        secret_stream: Some(SecretStreamInfo { version: 1 }),
        relay_through: None,
        relay_addresses: None,
    }
}

pub(super) async fn handle_handshake(
    svc: &Service,
    dedup: &mut Dedup,
    msg: HandshakeMessage,
    from: Ipv4Peer,
    peer_address: Option<Ipv4Peer>,
    reply_tx: tokio::sync::oneshot::Sender<Option<Vec<u8>>>,
) {
    tracing::debug!(
        mode = msg.mode,
        noise_len = msg.noise.len(),
        "relay: received handshake request"
    );
    let addresses4 = svc
        .dht
        .noise_addresses4(svc.dht.local_port().await.unwrap_or(0))
        .await;
    let gate = Gate {
        key_pair: &svc.key_pair,
        firewall: svc.firewall,
        peer_firewall: svc.peer_firewall.as_ref(),
    };
    let response = respond(
        dedup,
        &gate,
        &msg,
        &from,
        peer_address,
        addresses4,
        Instant::now(),
    );
    let _ = reply_tx.send(response.reply);
    if let Some(session) = response.session {
        svc.tasks.spawn(run_session(svc.clone(), session));
    }
}

/// Finalize one admitted control connection and drive its relay session.
async fn run_session(svc: Service, session: NewSession) {
    let finalized = finalize_relay_connection(
        svc.runtime_handle,
        &svc.dht,
        &svc.tasks,
        session.local_stream_id,
        &session.remote_udx,
        &session.noise,
    )
    .await;
    match finalized {
        Ok(mux) => {
            let remote_pk = session.noise.remote_public_key;
            run_relay_session(svc.relay, mux, remote_pk, svc.matched_tx).await;
        }
        Err(e) => {
            tracing::debug!(err = %e, "relay: connection finalize failed");
        }
    }
}

/// Finalize the accepted control connection: bind the raw UDX stream the
/// client dialed into, complete the Noise/SecretStream handshake, and wrap
/// the result in a Protomux [`Mux`] ready for a `"blind-relay"` channel.
///
/// Mirrors upstream `peeroxide::swarm::create_server_connection` (kept independent —
/// see module docs).
pub(super) async fn finalize_relay_connection(
    runtime_handle: Arc<RuntimeHandle>,
    dht: &HyperDhtHandle,
    tasks: &TaskSet,
    local_stream_id: u32,
    remote_udx: &UdxInfo,
    noise_result: &crate::noise_wrap::NoiseWrapResult,
) -> Result<Mux, HyperDhtError> {
    let runtime = UdxRuntime::shared(runtime_handle);

    let remote_id = u32::try_from(remote_udx.id)
        .map_err(|_| HyperDhtError::StreamEstablishment("remote UDX id out of u32 range".into()))?;

    let socket = dht.server_socket().await?.ok_or_else(|| {
        HyperDhtError::StreamEstablishment("DHT server socket not available".into())
    })?;

    let stream = runtime.create_stream(local_stream_id).await?;
    stream.set_firewall_hook(&socket, remote_id, |_, _, _| true)?;

    let async_stream = stream.into_async_stream();
    let ss = SecretStream::from_session(
        false,
        async_stream,
        noise_result.tx,
        noise_result.rx,
        noise_result.handshake_hash,
        noise_result.remote_public_key,
    )
    .await
    .map_err(HyperDhtError::SecretStream)?;

    let (mux, run) = Mux::new(ss);
    tasks.spawn(run);
    Ok(mux)
}

#[cfg(test)]
#[path = "handshake_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "admit_tests.rs"]
mod admit_tests;
