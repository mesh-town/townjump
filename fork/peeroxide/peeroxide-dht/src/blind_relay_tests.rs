//! Tests for the blind-relay wire format, client and server engine (protocol-only).
// JS: blind-relay/index.js

use super::test_support::{mem_pair, noop_outbound};
use super::*;
use crate::protomux::Mux;

#[test]
fn pair_roundtrip_initiator() {
    let msg = PairMessage {
        is_initiator: true,
        token: [0xaa; 32],
        id: 42,
        seq: 7,
    };
    let encoded = encode_pair_to_vec(&msg);
    let decoded = decode_pair_from_slice(&encoded).unwrap();
    assert_eq!(decoded, msg);
}

#[test]
fn pair_roundtrip_responder() {
    let msg = PairMessage {
        is_initiator: false,
        token: [0xbb; 32],
        id: 0,
        seq: 0,
    };
    let encoded = encode_pair_to_vec(&msg);
    let decoded = decode_pair_from_slice(&encoded).unwrap();
    assert_eq!(decoded, msg);
}

#[test]
fn unpair_roundtrip() {
    let msg = UnpairMessage { token: [0xcc; 32] };
    let encoded = encode_unpair_to_vec(&msg);
    let decoded = decode_unpair_from_slice(&encoded).unwrap();
    assert_eq!(decoded, msg);
}

#[test]
fn pair_wire_format() {
    let msg = PairMessage {
        is_initiator: true,
        token: [0x42; 32],
        id: 1,
        seq: 2,
    };
    let encoded = encode_pair_to_vec(&msg);

    assert_eq!(encoded[0], 0x01); // flags: bit0=1 (initiator)
    assert_eq!(&encoded[1..33], &[0x42; 32]); // token
    assert_eq!(encoded[33], 0x01); // id=1 (varint)
    assert_eq!(encoded[34], 0x02); // seq=2 (varint)
    assert_eq!(encoded.len(), 35);
}

#[test]
fn pair_wire_format_responder() {
    let msg = PairMessage {
        is_initiator: false,
        token: [0x00; 32],
        id: 0,
        seq: 0,
    };
    let encoded = encode_pair_to_vec(&msg);

    assert_eq!(encoded[0], 0x00); // flags: bit0=0 (responder)
    assert_eq!(&encoded[1..33], &[0x00; 32]); // token
    assert_eq!(encoded[33], 0x00); // id=0
    assert_eq!(encoded[34], 0x00); // seq=0
}

#[test]
fn unpair_wire_format() {
    let msg = UnpairMessage { token: [0xff; 32] };
    let encoded = encode_unpair_to_vec(&msg);

    assert_eq!(encoded[0], 0x00); // flags: all zero
    assert_eq!(&encoded[1..33], &[0xff; 32]); // token
    assert_eq!(encoded.len(), 33);
}

#[test]
fn pair_large_ids() {
    let msg = PairMessage {
        is_initiator: true,
        token: [0xde; 32],
        id: 100_000,
        seq: 65_536,
    };
    let encoded = encode_pair_to_vec(&msg);
    let decoded = decode_pair_from_slice(&encoded).unwrap();
    assert_eq!(decoded, msg);
}

#[test]
fn protocol_name_constant() {
    assert_eq!(PROTOCOL_NAME, "blind-relay");
}

#[tokio::test]
async fn client_pair_with_fake_relay() {
    let (stream_a, stream_b) = mem_pair();

    let (mux_a, run_a) = Mux::new(stream_a);
    let (mux_b, run_b) = Mux::new(stream_b);

    tokio::spawn(run_a);
    tokio::spawn(run_b);

    let token = [0xaa; 32];

    let client_task = tokio::spawn(async move {
        let mut client = BlindRelayClient::open(&mux_a, None).await.unwrap();
        client.wait_opened().await.unwrap();
        let resp = client.pair(true, &token, 42).await.unwrap();
        client.close();
        resp
    });

    // Fake relay server: open matching channel, wait for pair, send response
    let mut server_ch = mux_b
        .create_channel(PROTOCOL_NAME, None, None)
        .await
        .unwrap();
    server_ch.wait_opened().await.unwrap();

    let event = server_ch.recv().await.unwrap();
    match event {
        ChannelEvent::Message { message_type, data } => {
            assert_eq!(message_type, MSG_TYPE_PAIR);
            let pair_msg = decode_pair_from_slice(&data).unwrap();
            assert!(pair_msg.is_initiator);
            assert_eq!(pair_msg.token, token);
            assert_eq!(pair_msg.id, 42);

            let reply = PairMessage {
                is_initiator: true,
                token,
                id: 99,
                seq: 0,
            };
            server_ch
                .send(MSG_TYPE_PAIR, &encode_pair_to_vec(&reply))
                .unwrap();
        }
        other => panic!("expected pair Message, got {other:?}"),
    }

    let resp = client_task.await.unwrap();
    assert_eq!(resp.remote_id, 99);
}

#[tokio::test]
async fn client_unpair() {
    let (stream_a, stream_b) = mem_pair();

    let (mux_a, run_a) = Mux::new(stream_a);
    let (mux_b, run_b) = Mux::new(stream_b);

    tokio::spawn(run_a);
    tokio::spawn(run_b);

    let token = [0xbb; 32];

    let mut client = BlindRelayClient::open(&mux_a, None).await.unwrap();
    let mut server_ch = mux_b
        .create_channel(PROTOCOL_NAME, None, None)
        .await
        .unwrap();

    client.wait_opened().await.unwrap();
    server_ch.wait_opened().await.unwrap();

    client.unpair(&token).unwrap();

    let event = server_ch.recv().await.unwrap();
    match event {
        ChannelEvent::Message { message_type, data } => {
            assert_eq!(message_type, MSG_TYPE_UNPAIR);
            let unpair_msg = decode_unpair_from_slice(&data).unwrap();
            assert_eq!(unpair_msg.token, token);
        }
        other => panic!("expected unpair Message, got {other:?}"),
    }

    client.close();
}

// ── BlindRelayServer engine (protocol-only) ──────────────────────────

#[test]
fn try_pair_matches_two_sides() {
    let server = BlindRelayServer::new(BlindRelayServerConfig::default());
    let token = [0x11; 32];

    let first = server.try_pair(1, true, token, 100, 0, noop_outbound());
    assert!(matches!(first, PairOutcome::Pending));
    assert_eq!(server.stats().pairings_pending, 1);

    let second = server.try_pair(2, false, token, 200, 0, noop_outbound());
    match second {
        PairOutcome::Matched(m) => {
            assert_eq!(m.token, token);
            assert_eq!(m.first.session_id, 1);
            assert!(m.first.is_initiator);
            assert_eq!(m.first.client_stream_id, 100);
            assert_eq!(m.second.session_id, 2);
            assert!(!m.second.is_initiator);
            assert_eq!(m.second.client_stream_id, 200);
        }
        other => panic!("expected Matched, got {other:?}"),
    }

    let stats = server.stats();
    assert_eq!(stats.pairings_pending, 0);
    assert_eq!(stats.pairings_matched, 1);
    assert_eq!(stats.pairings_active, 1);
    assert_eq!(stats.pairings_requested, 2);
}

#[test]
fn try_pair_duplicate_same_slot_is_noop() {
    let server = BlindRelayServer::new(BlindRelayServerConfig::default());
    let token = [0x22; 32];

    assert!(matches!(
        server.try_pair(1, true, token, 1, 0, noop_outbound()),
        PairOutcome::Pending
    ));
    // Same session, same slot (initiator), sent again — Node treats
    // this as a silent no-op.
    assert!(matches!(
        server.try_pair(1, true, token, 1, 1, noop_outbound()),
        PairOutcome::AlreadyPairing
    ));
}

#[test]
fn try_pair_respects_max_pairings_per_session() {
    let config = BlindRelayServerConfig {
        max_pairings_per_session: 2,
        ..BlindRelayServerConfig::default()
    };
    let server = BlindRelayServer::new(config);

    assert!(matches!(
        server.try_pair(1, true, [0x01; 32], 1, 0, noop_outbound()),
        PairOutcome::Pending
    ));
    assert!(matches!(
        server.try_pair(1, true, [0x02; 32], 2, 1, noop_outbound()),
        PairOutcome::Pending
    ));
    // Session's 3rd concurrent pairing attempt exceeds the per-session cap.
    assert!(matches!(
        server.try_pair(1, true, [0x03; 32], 3, 2, noop_outbound()),
        PairOutcome::LimitExceeded
    ));
}

#[test]
fn try_accept_session_respects_max_sessions() {
    let config = BlindRelayServerConfig {
        max_sessions: 1,
        ..BlindRelayServerConfig::default()
    };
    let server = BlindRelayServer::new(config);

    let first = server.try_accept_session();
    assert!(first.is_some());
    assert!(server.try_accept_session().is_none());

    server.release_session(first.unwrap());
    assert!(server.try_accept_session().is_some());
}

#[test]
fn unpair_cancels_pending_registration() {
    let server = BlindRelayServer::new(BlindRelayServerConfig::default());
    let token = [0x33; 32];

    server.try_pair(1, true, token, 1, 0, noop_outbound());
    assert_eq!(server.stats().pairings_pending, 1);

    assert_eq!(server.unpair(1, token), UnpairOutcome::Cancelled);
    assert_eq!(server.stats().pairings_pending, 0);
    assert_eq!(server.stats().pairings_cancelled, 1);

    // Second unpair for the same (now-gone) token finds nothing.
    assert_eq!(server.unpair(1, token), UnpairOutcome::NotFound);
}

#[test]
fn unpair_unknown_token_not_found() {
    let server = BlindRelayServer::new(BlindRelayServerConfig::default());
    assert_eq!(server.unpair(1, [0x99; 32]), UnpairOutcome::NotFound);
}

#[test]
fn release_session_cancels_its_pending_pairings() {
    let server = BlindRelayServer::new(BlindRelayServerConfig::default());
    let token = [0x44; 32];

    let session_id = server.try_accept_session().unwrap();
    server.try_pair(session_id, true, token, 1, 0, noop_outbound());
    assert_eq!(server.stats().pairings_pending, 1);

    server.release_session(session_id);

    assert_eq!(server.stats().pairings_pending, 0);
    assert_eq!(server.stats().pairings_cancelled, 1);
    assert_eq!(server.stats().sessions_active, 0);
    // The token is fully free again for a fresh pairing attempt.
    assert!(matches!(
        server.try_pair(2, true, token, 1, 0, noop_outbound()),
        PairOutcome::Pending
    ));
}

#[test]
fn sweep_expired_pairings_drops_stale_entries() {
    let config = BlindRelayServerConfig {
        pairing_timeout: Duration::from_millis(1),
        ..BlindRelayServerConfig::default()
    };
    let server = BlindRelayServer::new(config);
    let token = [0x55; 32];

    server.try_pair(1, true, token, 1, 0, noop_outbound());
    std::thread::sleep(Duration::from_millis(20));

    let dropped = server.sweep_expired_pairings();
    assert_eq!(dropped, 1);
    assert_eq!(server.stats().pairings_pending, 0);
    assert_eq!(server.stats().pairings_cancelled, 1);

    // Free to re-register after the sweep.
    assert!(matches!(
        server.try_pair(2, true, token, 1, 0, noop_outbound()),
        PairOutcome::Pending
    ));
}

#[test]
fn sweep_expired_pairings_keeps_fresh_entries() {
    let config = BlindRelayServerConfig {
        pairing_timeout: Duration::from_secs(300),
        ..BlindRelayServerConfig::default()
    };
    let server = BlindRelayServer::new(config);
    server.try_pair(1, true, [0x66; 32], 1, 0, noop_outbound());

    assert_eq!(server.sweep_expired_pairings(), 0);
    assert_eq!(server.stats().pairings_pending, 1);
}
