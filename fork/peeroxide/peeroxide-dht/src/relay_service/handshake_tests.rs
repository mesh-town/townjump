//! Tests for [`super::respond`]: a repeated `PEER_HANDSHAKE` must not start a second session.

use std::time::{Duration, Instant};

use super::*;
use crate::hyperdht_messages::MODE_FROM_CLIENT;
use crate::noise::generate_keypair;

fn client_message(server: &KeyPair) -> HandshakeMessage {
    let mut client = NoiseWrap::new_initiator(generate_keypair(), server.public_key);
    let payload = NoisePayload {
        version: 1,
        error: 0,
        firewall: 0,
        holepunch: None,
        addresses4: vec![],
        addresses6: vec![],
        udx: Some(UdxInfo {
            version: 1,
            reusable_socket: true,
            id: 7,
            seq: 0,
        }),
        secret_stream: Some(SecretStreamInfo { version: 1 }),
        relay_through: None,
        relay_addresses: None,
    };
    HandshakeMessage {
        mode: MODE_FROM_CLIENT,
        noise: client.send(&payload).expect("client noise"),
        peer_address: None,
        relay_address: None,
    }
}

fn run(dedup: &mut Dedup, server: &KeyPair, msg: &HandshakeMessage, now: Instant) -> Response {
    respond(dedup, server, 0, msg, None, vec![], now)
}

#[test]
fn duplicate_peer_handshake_does_not_open_a_second_session() {
    let server = KeyPair::generate();
    let msg = client_message(&server);
    let mut dedup = Dedup::default();
    let now = Instant::now();

    let first = run(&mut dedup, &server, &msg, now);
    let second = run(&mut dedup, &server, &msg, now + Duration::from_secs(1));

    let sessions = [&first, &second]
        .iter()
        .filter(|r| r.session.is_some())
        .count();
    assert_eq!(sessions, 1, "one session for the same Noise message");
    assert!(first.reply.is_some());
    assert_eq!(
        second.reply, first.reply,
        "the repeat gets the stored reply"
    );
}

#[test]
fn noise_key_is_forgotten_after_clear_wait() {
    let server = KeyPair::generate();
    let msg = client_message(&server);
    let mut dedup = Dedup::default();
    let now = Instant::now();

    assert!(run(&mut dedup, &server, &msg, now).session.is_some());
    let later = run(&mut dedup, &server, &msg, now + Duration::from_secs(11));
    assert!(
        later.session.is_some(),
        "a key older than CLEAR_WAIT is new again"
    );
}
