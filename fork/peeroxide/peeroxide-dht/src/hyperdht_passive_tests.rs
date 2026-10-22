//! Unit test of `build_passive_holepunch_reply` (server side of a holepunch reply).

use super::*;
use crate::hyperdht_messages::{
    FIREWALL_CONSISTENT, FIREWALL_OPEN, HolepunchPayload, decode_holepunch_msg_from_bytes,
};
use crate::messages::Ipv4Peer;
use crate::secure_payload::SecurePayload;
use crate::socket_pool::SocketPool;
use libudx::UdxRuntime;
use tokio::sync::mpsc;

#[tokio::test]
async fn passive_holepunch_helper_roundtrip() {
    let holepunch_secret: [u8; 32] = [7u8; 32];

    let incoming_hp = HolepunchPayload {
        error: 0,
        firewall: FIREWALL_CONSISTENT,
        round: 0,
        connected: false,
        punching: true,
        addresses: Some(vec![Ipv4Peer {
            host: "10.0.0.1".to_string(),
            port: 5000,
        }]),
        remote_address: None,
        token: Some([0x42u8; 32]),
        remote_token: None,
    };

    let sp_sender = SecurePayload::new(holepunch_secret);
    let incoming_payload = sp_sender.encrypt(&incoming_hp).expect("encrypt incoming");

    let pool = SocketPool::new("0.0.0.0".to_string());
    let runtime = UdxRuntime::new().expect("UdxRuntime");
    let peer_addr = Ipv4Peer {
        host: "1.2.3.4".to_string(),
        port: 9876,
    };

    let (event_tx, _event_rx) = mpsc::unbounded_channel();
    let result = build_passive_holepunch_reply(
        FIREWALL_OPEN,
        &pool,
        &runtime,
        &peer_addr,
        holepunch_secret,
        &incoming_payload,
        0,
        event_tx,
        None,
    )
    .await
    .expect("build_passive_holepunch_reply should succeed");

    assert!(
        result.remote_hp.punching,
        "remote_hp.punching should be true"
    );
    assert_eq!(
        result.remote_hp.token,
        Some([0x42u8; 32]),
        "remote_hp.token should be present"
    );

    let decoded_msg =
        decode_holepunch_msg_from_bytes(&result.reply_bytes).expect("decode reply_bytes");

    let decoded_hp = result
        .payload
        .decrypt(&decoded_msg.payload)
        .expect("decrypt reply payload");

    assert!(
        !decoded_hp.connected,
        "decoded_hp.connected should be false"
    );
    assert!(decoded_hp.punching, "decoded_hp.punching should be true");
    assert_eq!(
        decoded_hp.remote_token,
        Some([0x42u8; 32]),
        "remote_token should be echoed in reply"
    );

    assert!(
        result.puncher.primary_socket().is_some(),
        "puncher should have a primary socket"
    );
}
