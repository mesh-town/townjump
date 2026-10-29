// JS: hyperdht/lib/connect.js (probeRound, isDone, updateHolepunch via c.puncher.socket)
//! Client fast mode: openSession, PEER_HOLEPUNCH from the puzzle socket, early Connected.
//! `Raced` / `or_connected` / `next_connected`: racing a step against the `Connected` event.

use std::net::SocketAddr;

use tokio::sync::mpsc;

use libudx::UdxSocket;

use crate::holepuncher::{HolepunchEvent, Holepuncher};
use crate::hyperdht_messages::{
    ERROR_ABORTED, ERROR_NONE, FIREWALL_UNKNOWN, HolepunchPayload, NoisePayload, PEER_HOLEPUNCH,
};
use crate::messages::Ipv4Peer;
use crate::noise_wrap::NoiseWrapResult;
use crate::rpc::{DhtError, DhtHandle, ResponseData, UserRequestParams};

use super::connection::ConnectResult;
use super::types::HyperDhtError;

/// One low-TTL packet to the server's puzzle address: opens our NAT for its ping.
pub(super) fn open_session(puncher: &Holepuncher, server_address: &Ipv4Peer) {
    let Some(sr) = puncher.primary_socket() else {
        return;
    };
    if let Err(e) = sr.send_holepunch_to(&server_address.host, server_address.port, true) {
        tracing::debug!(err = %e, "open_session failed");
    }
}

/// `Connected` already waiting in the event channel (JS `isDone(c)`), if any: the address and the
/// socket that received the probe.
pub(super) fn connected_now(
    rx: &mut mpsc::UnboundedReceiver<HolepunchEvent>,
) -> Option<(SocketAddr, UdxSocket)> {
    match rx.try_recv() {
        Ok(HolepunchEvent::Connected { addr, socket }) => Some((addr, socket)),
        _ => None,
    }
}

/// ConnectResult over `udx_socket`: the socket that received the peer's probe; logs "punch successful" (both the early and the final path).
pub(super) fn punched_result(
    nw_result: &NoiseWrapResult,
    remote_payload: &NoisePayload,
    local_stream_id: u32,
    addr: SocketAddr,
    udx_socket: Option<UdxSocket>,
) -> ConnectResult {
    tracing::info!(
        target: "peeroxide::_events::holepunch::connected",
        from = %addr,
        "punch successful"
    );
    let connected_addr = Ipv4Peer {
        host: addr.ip().to_string(),
        port: addr.port(),
    };
    ConnectResult {
        remote_public_key: nw_result.remote_public_key,
        server_address: connected_addr.clone(),
        client_address: connected_addr,
        is_relayed: true,
        noise: nw_result.clone(),
        local_stream_id,
        remote_udx: remote_payload.udx.clone(),
        udx_socket,
    }
}

/// PEER_HOLEPUNCH to `relay`: from the puzzle socket (`via`) when there is one.
/// Replies on `via` reach `dht` only through the forwarder that `Holepuncher::start_background_sampling` (client)
/// or `Holepuncher::auto_sample` (server) starts: call it first.
pub(super) async fn holepunch_request(
    dht: &DhtHandle,
    target: &[u8; 32],
    value: Vec<u8>,
    relay: &Ipv4Peer,
    via: Option<UdxSocket>,
) -> Result<ResponseData, DhtError> {
    let params = UserRequestParams {
        token: None,
        command: PEER_HOLEPUNCH,
        target: Some(*target),
        value: Some(value),
        timeout_ms: None,
        retries: None,
    };
    match via {
        Some(via) => dht.request_via(params, &relay.host, relay.port, via).await,
        None => dht.request(params, &relay.host, relay.port).await,
    }
}

/// A `punching=false` probe that names the server's puzzle address (`remote_address`).
pub(super) fn build_probe(
    firewall: u64,
    round: u64,
    addresses: Vec<Ipv4Peer>,
    server_address: &Ipv4Peer,
    token: [u8; 32],
    remote_token: Option<[u8; 32]>,
) -> HolepunchPayload {
    HolepunchPayload {
        error: ERROR_NONE,
        firewall,
        round,
        connected: false,
        punching: false,
        addresses: Some(addresses),
        remote_address: Some(server_address.clone()),
        token: Some(token),
        remote_token,
    }
}

/// JS `probeRound`: open the session first, then send the probe.
pub(super) async fn send_probe(
    dht: &DhtHandle,
    puncher: &Holepuncher,
    server_address: &Ipv4Peer,
    target: &[u8; 32],
    value: Vec<u8>,
    relay: &Ipv4Peer,
    via: Option<UdxSocket>,
) -> Result<ResponseData, DhtError> {
    open_session(puncher, server_address);
    holepunch_request(dht, target, value, relay, via).await
}

/// JS `connect.js:623`: the probe reply feeds NAT with our reflexive address (`to`) as seen
/// by the relay, not with the server's address. Without `to` no sample is added.
pub(crate) fn add_probe_sample(nat: &mut crate::nat::Nat, resp: &ResponseData) {
    if let Some(to) = &resp.to {
        nat.add(to, &resp.from);
    }
}

/// A server `ERROR_ABORTED` in the final reply ends the connect at once (as in the probe loop).
pub(super) fn final_reply_error(error: u64) -> Result<(), HyperDhtError> {
    match error {
        ERROR_ABORTED => Err(HyperDhtError::HolepunchAborted),
        _ => Ok(()),
    }
}

/// Final-round payload (`punching: true`), sent once the remote address is verified.
pub(super) fn build_punch(
    firewall: u64,
    round: u64,
    addresses: Vec<Ipv4Peer>,
    verified_addr: &Ipv4Peer,
    token: [u8; 32],
    remote_token: Option<[u8; 32]>,
) -> HolepunchPayload {
    HolepunchPayload {
        error: ERROR_NONE,
        firewall,
        round,
        connected: false,
        punching: true,
        addresses: Some(addresses),
        remote_address: Some(verified_addr.clone()),
        token: Some(token),
        remote_token,
    }
}

/// Waits for the next event: `Connected` gives the address; `Aborted` or a closed channel is an error.
pub(super) async fn next_connected(
    rx: &mut mpsc::UnboundedReceiver<HolepunchEvent>,
) -> Result<(SocketAddr, UdxSocket), HyperDhtError> {
    match rx.recv().await {
        Some(HolepunchEvent::Connected { addr, socket }) => Ok((addr, socket)),
        _ => {
            tracing::info!(
                target: "peeroxide::_events::holepunch::aborted",
                "punch aborted"
            );
            Err(HyperDhtError::HolepunchAborted)
        }
    }
}

/// Outcome of a step racing the `Connected` event.
pub(super) enum Raced<T> {
    Done(T),
    Connected(SocketAddr, UdxSocket),
}

/// Runs `fut` (final-round request, `analyze`), but a `Connected` event ends it without waiting;
/// `Aborted` or a closed channel is an error.
pub(super) async fn or_connected<T, F>(
    fut: F,
    rx: &mut mpsc::UnboundedReceiver<HolepunchEvent>,
) -> Result<Raced<T>, HyperDhtError>
where
    F: std::future::Future<Output = T>,
{
    // `biased`: when both are ready, `Connected` wins (the result no longer matters).
    tokio::select! {
        biased;
        c = next_connected(rx) => {
            let (addr, socket) = c?;
            Ok(Raced::Connected(addr, socket))
        }
        r = fut => Ok(Raced::Done(r)),
    }
}

/// JS `connect.js:640-643`: 1 s grace while the remote firewall is still UNKNOWN.
pub(super) async fn grace_for_unknown(firewall: u64) {
    if firewall == FIREWALL_UNKNOWN {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}

#[cfg(test)]
#[path = "connect_fast_open_tests.rs"]
mod tests;
