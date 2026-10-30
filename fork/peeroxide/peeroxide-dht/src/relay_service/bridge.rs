//! Data-plane bridging of matched pairings (raw UDX streams, blind relay).
//! Siblings: `accept`, `session`.
// JS: blind-relay/index.js

use std::collections::HashMap;
use std::sync::Arc;

use libudx::{RuntimeHandle, UdxRuntime, UdxSocket, UdxStream};
use tokio::sync::mpsc;

use crate::blind_relay::{BlindRelayServer, MatchedPairing, SessionOutbound};
use crate::hyperdht::{HyperDhtError, alloc_stream_id};

use super::session::SessionSockets;
use super::stop::TaskSet;

/// Shared storage for bridged (active) pairings' raw stream pairs, keyed
/// by relay token.
pub(super) type ActiveStreamMap =
    Arc<tokio::sync::Mutex<HashMap<[u8; 32], (UdxStream, UdxStream)>>>;

/// Consume matched pairings, creating and bridging the two raw UDX
/// data-plane streams for each (mirrors Node's `createStream()` +
/// `stream.relayTo(remote.stream)`, both directions).
///
/// Bridged streams are kept in `active`, keyed by token, until an explicit
/// teardown signal arrives (from `BlindRelayServer::unpair` on an active
/// pairing, or `release_session` when either side's session closes —
/// both registered via `BlindRelayServer::mark_active`). Dropping a
/// `UdxStream` aborts its packet-forwarding task, so removing an entry here is enough to stop forwarding and
/// free the resources. Each pair's task belongs to the service's `tasks`, so
/// stopping the service drops every bridged pair too.
pub(super) async fn bridge_matched_pairings(
    runtime_handle: Arc<RuntimeHandle>,
    relay: BlindRelayServer,
    mut matched_rx: mpsc::UnboundedReceiver<MatchedPairing>,
    tasks: TaskSet,
    sockets: SessionSockets,
) {
    let active: ActiveStreamMap = Arc::new(tokio::sync::Mutex::new(HashMap::new()));

    while let Some(matched) = matched_rx.recv().await {
        let runtime_handle = Arc::clone(&runtime_handle);
        let sockets = sockets.clone();
        let relay = relay.clone();
        let active = Arc::clone(&active);
        tasks.spawn(async move {
            let token = matched.token;
            let session_ids = [matched.first.session_id, matched.second.session_id];
            match bridge_one_pairing(runtime_handle, &sockets, &relay, matched).await {
                Ok(pair) => hold_pair(&active, &relay, token, session_ids, pair).await,
                Err(e) => {
                    tracing::debug!(err = %e, "relay: failed to bridge matched pairing");
                }
            }
        });
    }
}

/// Register an opened pair as active and keep it until the teardown signal
/// (unpair from either side, or a session closing), then drop both streams
/// (stopping forwarding) and free the map entry.
pub(super) async fn hold_pair(
    active: &ActiveStreamMap,
    relay: &BlindRelayServer,
    token: [u8; 32],
    session_ids: [u64; 2],
    pair: (UdxStream, UdxStream),
) {
    active.lock().await.insert(token, pair);
    let (teardown_tx, mut teardown_rx) = mpsc::unbounded_channel::<()>();
    relay.mark_active(token, session_ids, teardown_tx);

    let _ = teardown_rx.recv().await;
    active.lock().await.remove(&token);
    relay.note_stream_closed();
    relay.note_stream_closed();
}

pub(super) async fn bridge_one_pairing(
    runtime_handle: Arc<RuntimeHandle>,
    sockets: &SessionSockets,
    relay: &BlindRelayServer,
    matched: MatchedPairing,
) -> Result<(UdxStream, UdxStream), HyperDhtError> {
    let runtime = UdxRuntime::shared(runtime_handle);
    let (Some(first_socket), Some(second_socket)) = (
        sockets.get(matched.first.session_id),
        sockets.get(matched.second.session_id),
    ) else {
        relay.note_stream_error();
        return Err(HyperDhtError::StreamEstablishment(
            "control connection socket not available".into(),
        ));
    };

    let first_local_id = alloc_stream_id();
    let second_local_id = alloc_stream_id();
    let (first_stream, second_stream) = open_pair(
        &runtime,
        [&first_socket, &second_socket],
        relay,
        [first_local_id, second_local_id],
        [
            matched.first.client_stream_id,
            matched.second.client_stream_id,
        ],
    )
    .await?;

    let _ = matched
        .first
        .outbound_tx
        .send(SessionOutbound::PairMatched {
            token: matched.token,
            is_initiator: matched.first.is_initiator,
            local_stream_id: u64::from(first_local_id),
        });
    let _ = matched
        .second
        .outbound_tx
        .send(SessionOutbound::PairMatched {
            token: matched.token,
            is_initiator: matched.second.is_initiator,
            local_stream_id: u64::from(second_local_id),
        });

    Ok((first_stream, second_stream))
}

/// Create the two raw streams, bind each to its client's stream id and relay
/// packets between them in both directions. Counts opened streams and errors.
pub(super) async fn open_pair(
    runtime: &UdxRuntime,
    sockets: [&UdxSocket; 2],
    relay: &BlindRelayServer,
    local_ids: [u32; 2],
    client_ids: [u64; 2],
) -> Result<(UdxStream, UdxStream), HyperDhtError> {
    let opened = wire_pair(runtime, sockets, relay, local_ids, client_ids).await;
    if opened.is_err() {
        relay.note_stream_error();
    }
    opened
}

async fn wire_pair(
    runtime: &UdxRuntime,
    sockets: [&UdxSocket; 2],
    relay: &BlindRelayServer,
    local_ids: [u32; 2],
    client_ids: [u64; 2],
) -> Result<(UdxStream, UdxStream), HyperDhtError> {
    let first_remote_id = remote_id(client_ids[0])?;
    let second_remote_id = remote_id(client_ids[1])?;

    let first_stream = runtime.create_stream(local_ids[0]).await?;
    relay.note_stream_opened();
    first_stream.set_firewall_hook(sockets[0], first_remote_id, |_, _, _| true)?;

    let second_stream = runtime.create_stream(local_ids[1]).await?;
    relay.note_stream_opened();
    second_stream.set_firewall_hook(sockets[1], second_remote_id, |_, _, _| true)?;

    // Blind, bidirectional packet-level forwarding — peeroxide never reads
    // or decrypts the relayed application data. Safe to wire up before
    // either side's first packet arrives: `UdxStream::process_incoming`
    // runs the firewall-hook gate (single-fire 4-tuple adoption) *before*
    // checking `relay_target`, so the packet that fires a stream's hook
    // still gets forwarded on this same pass rather than being absorbed
    // into normal (unread) stream processing — see the ordering comment
    // in `libudx::native::stream::process_incoming`.
    first_stream.relay_to(&second_stream)?;
    second_stream.relay_to(&first_stream)?;
    Ok((first_stream, second_stream))
}

fn remote_id(client_stream_id: u64) -> Result<u32, HyperDhtError> {
    u32::try_from(client_stream_id)
        .map_err(|_| HyperDhtError::StreamEstablishment("client stream id out of range".into()))
}

#[cfg(test)]
#[path = "bridge_tests.rs"]
mod tests;
