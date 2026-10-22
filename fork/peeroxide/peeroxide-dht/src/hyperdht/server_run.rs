//! Server actor: `ServerSession`, `run_server` loop and `handle_server_handshake`.
//! Neighbours: `server_passive` builds the holepunch reply; `node` routes peer requests to the actor.

use tokio::sync::mpsc;

use libudx::UdxRuntime;

use crate::hyperdht_messages::{HandshakeMessage, NoisePayload, SecretStreamInfo, UdxInfo};
use crate::messages::Ipv4Peer;
use crate::noise_wrap::NoiseWrap;
use crate::peer::NodeId;
use crate::rpc::DhtHandle;
use crate::secure_payload::SecurePayload;
use crate::socket_pool::SocketPool;

use super::config::ServerConfig;
use super::server_passive::build_passive_holepunch_reply;
use super::stream::next_stream_id;
use super::types::ServerEvent;

// ── Server-side event handler ─────────────────────────────────────────────────

/// Per-server state for pending handshake and holepunch exchanges.
pub struct ServerSession {
    /// Cached holepunch secrets indexed by remote public key.
    holepunch_secrets: std::collections::HashMap<[u8; 32], ServerPeerState>,
}

#[allow(dead_code)]
struct ServerPeerState {
    holepunch_secret: [u8; 32],
    remote_public_key: [u8; 32],
    client_address: Ipv4Peer,
    local_stream_id: u32,
    remote_udx: Option<UdxInfo>,
}

/// Run the server-side request loop for peer handshakes and holepunches.
pub async fn run_server(
    mut event_rx: mpsc::UnboundedReceiver<ServerEvent>,
    config: ServerConfig,
    runtime: UdxRuntime,
    dht: DhtHandle,
) {
    let mut session = ServerSession {
        holepunch_secrets: std::collections::HashMap::new(),
    };
    let pool = SocketPool::new("0.0.0.0".into());

    while let Some(event) = event_rx.recv().await {
        match event {
            ServerEvent::PeerHandshake {
                msg,
                from,
                peer_address: _,
                target,
                reply_tx,
                ..
            } => {
                let reply =
                    handle_server_handshake(&config, &mut session, msg, &from, target.as_ref());
                let _ = reply_tx.send(reply);
            }
            ServerEvent::PeerHolepunch {
                msg,
                from: _,
                peer_address,
                target: _,
                reply_tx,
            } => {
                let holepunch_secret = session.holepunch_secrets.values().find_map(|state| {
                    let sp = SecurePayload::new(state.holepunch_secret);
                    if sp.decrypt(&msg.payload).is_ok() {
                        Some(state.holepunch_secret)
                    } else {
                        None
                    }
                });
                let reply: Option<Vec<u8>> = if let Some(secret) = holepunch_secret {
                    let (event_tx, _event_rx) = mpsc::unbounded_channel();
                    match build_passive_holepunch_reply(
                        config.firewall,
                        &pool,
                        &runtime,
                        &peer_address,
                        secret,
                        &msg.payload,
                        msg.id,
                        event_tx,
                        Some(&dht),
                    )
                    .await
                    {
                        Ok(result) => {
                            if result.remote_hp.punching {
                                let pool_clone = SocketPool::new("0.0.0.0".into());
                                let mut puncher = result.puncher;
                                tokio::spawn(async move {
                                    if let Ok(rt) = UdxRuntime::new() {
                                        puncher.punch(&pool_clone, &rt).await;
                                    }
                                });
                            }
                            Some(result.reply_bytes)
                        }
                        Err(_) => None,
                    }
                } else {
                    None
                };
                let _ = reply_tx.send(reply);
            }
        }
    }
}

fn handle_server_handshake(
    config: &ServerConfig,
    session: &mut ServerSession,
    msg: HandshakeMessage,
    from: &Ipv4Peer,
    _target: Option<&NodeId>,
) -> Option<Vec<u8>> {
    let mut nw = NoiseWrap::new_responder(config.key_pair.to_noise_keypair());

    let remote_payload = match nw.recv(&msg.noise) {
        Ok(p) => p,
        Err(_) => return None,
    };

    if remote_payload.error != 0 {
        return None;
    }

    let local_stream_id = next_stream_id();

    let reply_payload = NoisePayload {
        version: 1,
        error: 0,
        firewall: config.firewall,
        holepunch: None,
        addresses4: vec![],
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
    };

    let noise_reply = match nw.send(&reply_payload) {
        Ok(b) => b,
        Err(_) => return None,
    };

    let nw_result = match nw.finalize() {
        Ok(r) => r,
        Err(_) => return None,
    };

    session.holepunch_secrets.insert(
        nw_result.remote_public_key,
        ServerPeerState {
            holepunch_secret: nw_result.holepunch_secret,
            remote_public_key: nw_result.remote_public_key,
            client_address: from.clone(),
            local_stream_id,
            remote_udx: remote_payload.udx.clone(),
        },
    );

    let reply_msg = HandshakeMessage {
        mode: crate::hyperdht_messages::MODE_REPLY,
        noise: noise_reply,
        peer_address: Some(from.clone()),
        relay_address: None,
    };

    crate::hyperdht_messages::encode_handshake_to_bytes(&reply_msg).ok()
}
