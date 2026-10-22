//! The client's first Noise payload (PEER_HANDSHAKE message 1), a pure function of the connect inputs.
// JS: hyperdht/lib/connect.js:455

use crate::hyperdht_messages::{
    FIREWALL_UNKNOWN, NoisePayload, RelayThroughInfo, SecretStreamInfo, UdxInfo,
};
use crate::messages::Ipv4Peer;

use super::config::ConnectOpts;

/// Build the payload the client sends in its first handshake message.
pub(super) fn client_payload(
    addresses4: Vec<Ipv4Peer>,
    local_stream_id: u32,
    opts: &ConnectOpts,
) -> NoisePayload {
    let relay_through = opts.relay_ask.as_ref().map(|a| RelayThroughInfo {
        version: 1,
        public_key: a.public_key,
        token: a.token,
    });
    NoisePayload {
        version: 1,
        error: 0,
        firewall: FIREWALL_UNKNOWN,
        holepunch: None,
        addresses4,
        addresses6: vec![],
        udx: Some(UdxInfo {
            version: 1,
            reusable_socket: opts.reusable_socket,
            id: u64::from(local_stream_id),
            seq: 0,
        }),
        secret_stream: Some(SecretStreamInfo { version: 1 }),
        relay_through,
        relay_addresses: None,
    }
}

#[cfg(test)]
#[path = "connect_payload_tests.rs"]
mod tests;
