// JS: hyperdht/lib/server.js
//! Pure builders of the server's answers: Noise payload, holepunch round
//! payloads, and the wire messages that carry them (JS lines 376-390, 516-657).

use crate::hyperdht_messages::{
    self, ERROR_ABORTED, ERROR_NONE, ERROR_VERSION_MISMATCH, FIREWALL_OPEN, FIREWALL_UNKNOWN,
    HandshakeMessage, HolepunchInfo, HolepunchMessage, HolepunchPayload, MODE_FROM_RELAY,
    MODE_FROM_SECOND_RELAY, MODE_FROM_SERVER, MODE_REPLY, NoisePayload, RelayInfo,
    RelayThroughInfo, SecretStreamInfo, UdxInfo,
};
use crate::messages::Ipv4Peer;

/// How many relay addresses go into `relay_addresses` (JS announcer cap).
const MAX_RELAY_ADDRESSES: usize = 3;

/// Our side of the Noise payload.
pub(super) struct Ours {
    /// `ERROR_*` code (`ERROR_NONE` for a normal answer).
    pub(super) error: u64,
    /// Our `FIREWALL_*` type.
    pub(super) firewall: u64,
    /// Addresses the client can reach us on.
    pub(super) addresses4: Vec<Ipv4Peer>,
    /// Holepunch session id (used when we are not open).
    pub(super) holepunch_id: u64,
    /// Relays of the announcement: `peer_address` is our address seen by the relay.
    pub(super) relays: Vec<RelayInfo>,
    /// Our UDX stream id (`0` when no stream was created).
    pub(super) udx_id: u64,
    /// The `reusable_socket` option of the server.
    pub(super) reusable_socket: bool,
    /// The relay we announce (`None`: no `relay_through` in the payload).
    pub(super) relay_through: Option<RelayThroughInfo>,
}

/// Reply mode: through a relay the answer travels back as `FROM_SERVER`
/// (with the request's `peer_address`), a direct one is a plain `REPLY`.
pub(super) fn reply_mode(inbound: u64, peer_address: Option<&Ipv4Peer>) -> (u64, Option<Ipv4Peer>) {
    match inbound {
        MODE_FROM_RELAY | MODE_FROM_SECOND_RELAY => (MODE_FROM_SERVER, peer_address.cloned()),
        _ => (MODE_REPLY, None),
    }
}

/// Error code for the client's Noise payload: version first, then missing UDX info.
pub(super) fn reply_error(remote: &NoisePayload) -> u64 {
    if remote.version != 1 {
        return ERROR_VERSION_MISMATCH;
    }
    if remote.udx.is_none() {
        return ERROR_ABORTED;
    }
    ERROR_NONE
}

/// The Noise payload we send in the handshake reply.
pub(super) fn handshake_payload(o: &Ours) -> NoisePayload {
    NoisePayload {
        version: 1,
        error: o.error,
        firewall: o.firewall,
        holepunch: holepunch_info(o),
        addresses4: o.addresses4.clone(),
        addresses6: vec![],
        udx: Some(udx_info(o)),
        secret_stream: Some(SecretStreamInfo { version: 1 }),
        relay_through: o.relay_through.clone(),
        relay_addresses: relay_addresses(&o.relays),
    }
}

fn holepunch_info(o: &Ours) -> Option<HolepunchInfo> {
    (o.firewall != FIREWALL_OPEN).then(|| HolepunchInfo {
        id: o.holepunch_id,
        relays: o.relays.clone(),
    })
}

fn udx_info(o: &Ours) -> UdxInfo {
    UdxInfo {
        version: 1,
        reusable_socket: o.reusable_socket,
        id: o.udx_id,
        seq: 0,
    }
}

fn relay_addresses(relays: &[RelayInfo]) -> Option<Vec<Ipv4Peer>> {
    let first = relays.iter().take(MAX_RELAY_ADDRESSES);
    let addresses: Vec<Ipv4Peer> = first.map(|r| r.relay_address.clone()).collect();
    (!addresses.is_empty()).then_some(addresses)
}

/// Wire bytes of the handshake reply to `inbound` (mode and `peer_address` as in the request).
pub(super) fn encode_reply(inbound: &HandshakeMessage, noise: Vec<u8>) -> Option<Vec<u8>> {
    let (mode, peer_address) = reply_mode(inbound.mode, inbound.peer_address.as_ref());
    let reply = HandshakeMessage {
        mode,
        noise,
        peer_address,
        relay_address: None,
    };
    hyperdht_messages::encode_handshake_to_bytes(&reply).ok()
}

/// Payload of a normal holepunch round answer (`connected` is always false).
pub(super) fn round_payload(
    error: u64,
    firewall: u64,
    round: u64,
    punching: bool,
    addresses: Vec<Ipv4Peer>,
    token: Option<[u8; 32]>,
    remote_token: Option<[u8; 32]>,
) -> HolepunchPayload {
    HolepunchPayload {
        error,
        firewall,
        round,
        connected: false,
        punching,
        addresses: Some(addresses),
        remote_address: None,
        token,
        remote_token,
    }
}

/// Payload that tells the client to give up (JS `_abort`).
pub(super) fn abort_payload(round: u64) -> HolepunchPayload {
    HolepunchPayload {
        error: ERROR_ABORTED,
        firewall: FIREWALL_UNKNOWN,
        round,
        connected: false,
        punching: false,
        addresses: None,
        remote_address: None,
        token: None,
        remote_token: None,
    }
}

/// Wire bytes of the holepunch answer to `inbound`, carrying the encrypted `payload`.
pub(super) fn encode_holepunch(
    inbound: &HolepunchMessage,
    peer_address: &Ipv4Peer,
    payload: Vec<u8>,
) -> Option<Vec<u8>> {
    let (mode, _) = reply_mode(inbound.mode, Some(peer_address));
    let reply = HolepunchMessage {
        mode,
        id: inbound.id,
        payload,
        peer_address: Some(peer_address.clone()),
    };
    hyperdht_messages::encode_holepunch_msg_to_bytes(&reply).ok()
}

#[cfg(test)]
#[path = "reply_tests.rs"]
mod tests;
