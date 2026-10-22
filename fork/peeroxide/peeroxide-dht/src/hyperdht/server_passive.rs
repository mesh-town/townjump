//! `build_passive_holepunch_reply`: the server side of a holepunch reply.
//! Neighbours: `server_run` and `node_holepunch` call it.

use tokio::sync::mpsc;

use libudx::UdxRuntime;

use crate::holepuncher::{HolepunchEvent, Holepuncher};
use crate::hyperdht_messages::{HolepunchMessage, HolepunchPayload};
use crate::messages::Ipv4Peer;
use crate::secure_payload::SecurePayload;
use crate::socket_pool::SocketPool;

use super::types::HyperDhtError;

/// Result returned by [`build_passive_holepunch_reply`].
#[non_exhaustive]
pub struct PassiveHolepunchReply {
    /// Fully-encoded [`HolepunchMessage`] bytes ready to send on the wire.
    pub reply_bytes: Vec<u8>,
    /// Decrypted incoming holepunch payload from the remote peer.
    pub remote_hp: HolepunchPayload,
    /// The [`SecurePayload`] instance used for this exchange; reuse it across rounds for token continuity.
    pub payload: SecurePayload,
    /// Constructed [`Holepuncher`]; caller decides whether to spawn `punch()` or retain it.
    pub puncher: Holepuncher,
}

/// Build a passive (server-side) holepunch reply from raw encrypted bytes.
///
/// Pass `dht = Some(&dht_handle)` to run the puncher's autoSample
/// before returning (required for the `coerce_firewall` revert to be
/// safe on the passive path). Tests may pass `None` to skip sampling.
#[allow(clippy::too_many_arguments)]
pub async fn build_passive_holepunch_reply(
    config_firewall: u64,
    pool: &SocketPool,
    runtime: &UdxRuntime,
    peer_address: &Ipv4Peer,
    holepunch_secret: [u8; 32],
    incoming_payload: &[u8],
    msg_id: u64,
    event_tx: mpsc::UnboundedSender<HolepunchEvent>,
    dht: Option<&crate::rpc::DhtHandle>,
) -> Result<PassiveHolepunchReply, HyperDhtError> {
    let sp = SecurePayload::new(holepunch_secret);
    let remote_hp = sp.decrypt(incoming_payload)?;

    let mut puncher = Holepuncher::new(pool, runtime, true, false, remote_hp.firewall, event_tx)
        .await
        .map_err(|_| HyperDhtError::HolepunchFailed)?;

    if let Some(addrs) = &remote_hp.addresses {
        puncher.update_remote(
            true,
            remote_hp.firewall,
            addrs,
            Some(peer_address.host.as_str()),
        );
    }

    // Seed the passive puncher's NAT classifier BEFORE the caller spawns
    // punch(). Symmetric to the initiator path — without this the passive
    // NAT stays UNKNOWN and the `coerce_firewall` deviation from JS would
    // report a wrong state. This is what makes the `coerce_firewall` revert
    // safe on the passive path.
    if let Some(dht_handle) = dht {
        let added = puncher.auto_sample(dht_handle).await;
        tracing::debug!(
            added,
            firewall = puncher.nat.firewall,
            "passive auto_sample"
        );
    }

    // Advertise the passive puncher's reachable addresses so the initiator
    // probes back at us: only the autoSample-derived reflexive samples
    // (`puncher.nat.addresses`, Node `js/server.js` `p.nat.addresses`), no
    // interface addresses. Echoing `peer_address` here would tell the
    // initiator to punch its own reflexive address.
    let local_punch_addrs = puncher.punch_addresses();

    let reply_hp = HolepunchPayload {
        error: 0,
        firewall: config_firewall,
        round: remote_hp.round,
        connected: false,
        punching: remote_hp.punching,
        addresses: Some(local_punch_addrs),
        remote_address: Some(peer_address.clone()),
        token: Some(sp.token(&peer_address.host)),
        remote_token: remote_hp.token,
    };

    let encrypted_reply = sp.encrypt(&reply_hp)?;

    let reply_msg = HolepunchMessage {
        mode: crate::hyperdht_messages::MODE_REPLY,
        id: msg_id,
        payload: encrypted_reply,
        peer_address: Some(peer_address.clone()),
    };

    let reply_bytes = crate::hyperdht_messages::encode_holepunch_msg_to_bytes(&reply_msg)?;

    Ok(PassiveHolepunchReply {
        reply_bytes,
        remote_hp,
        payload: sp,
        puncher,
    })
}
