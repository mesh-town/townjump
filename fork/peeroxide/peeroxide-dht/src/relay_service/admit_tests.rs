//! Admission of the relay service: `peer_firewall` refuses silently (no reply, no session, no
//! pairing), a listed key is served as before, and one key may hold several sessions.
// JS: blind-relay/index.js

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::*;
use crate::hyperdht::{HyperDhtError, RelayPaired, pair_via_relay};
use crate::hyperdht_messages::MODE_FROM_CLIENT;
use crate::messages::Ipv4Peer;
use crate::noise::{Keypair as NoiseKeypair, generate_keypair};
use crate::relay_service::RelayServiceConfig;
use crate::relay_service::test_net::{RelayNet, relay_net};
use crate::server::{Firewall, FirewallInput};

mod peer_outside_the_list {
    use super::*;

    #[test]
    fn gets_no_reply_and_no_session() {
        let (listed, stranger) = (generate_keypair(), generate_keypair());
        let mut relay = Relay::new(Some(only(&[listed.public_key])));

        let response = relay.handshake(&stranger);

        assert!(response.reply.is_none(), "a refused peer gets silence");
        assert!(response.session.is_none(), "a refused peer gets no session");
    }

    #[test]
    fn repeating_the_message_stays_silent_and_asks_the_callback_once() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&calls);
        let refuse_all: Firewall = Arc::new(move |_| {
            counted.fetch_add(1, Ordering::SeqCst);
            true
        });
        let mut relay = Relay::new(Some(refuse_all));
        let msg = client_message(&relay.server, &generate_keypair());

        let first = relay.respond_to(&msg, None);
        let second = relay.respond_to(&msg, None);

        assert!(first.reply.is_none() && first.session.is_none());
        assert!(second.reply.is_none() && second.session.is_none());
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "the repeat is not asked again"
        );
    }
}

mod peer_on_the_list {
    use super::*;

    #[test]
    fn gets_a_reply_and_a_session() {
        let listed = generate_keypair();
        let mut relay = Relay::new(Some(only(&[listed.public_key])));

        let response = relay.handshake(&listed);

        assert!(response.reply.is_some(), "a listed peer is answered");
        assert!(response.session.is_some(), "a listed peer gets a session");
    }
}

mod callback_panics {
    use super::*;

    #[test]
    fn refuses_the_peer_and_serves_the_next_one() {
        let (bad, good) = (generate_keypair(), generate_keypair());
        let bad_key = bad.public_key;
        let explode_on_bad: Firewall = Arc::new(move |input| {
            if *input.remote_public_key == bad_key {
                panic!("firewall callback failed");
            }
            false
        });
        let mut relay = Relay::new(Some(explode_on_bad));

        let refused = relay.handshake(&bad);
        let served = relay.handshake(&good);

        assert!(refused.reply.is_none(), "a panic counts as a refusal");
        assert!(refused.session.is_none());
        assert!(served.reply.is_some(), "the service survives the panic");
    }
}

mod without_a_firewall {
    use super::*;

    #[test]
    fn admits_everyone() {
        let mut relay = Relay::new(None);

        let response = relay.handshake(&generate_keypair());

        assert!(response.reply.is_some() && response.session.is_some());
    }
}

mod firewall_input {
    use super::*;

    #[test]
    fn carries_the_client_key_and_the_peer_address() {
        let client = generate_keypair();
        let (firewall, seen) = recording();
        let mut relay = Relay::new(Some(firewall));
        let msg = client_message(&relay.server, &client);
        let behind_relay = Ipv4Peer {
            host: "10.9.8.7".into(),
            port: 6001,
        };

        relay.respond_to(&msg, Some(behind_relay));

        let seen = seen.lock().expect("seen");
        assert_eq!(
            *seen,
            vec![(client.public_key, "10.9.8.7".to_string(), 6001)]
        );
    }

    #[test]
    fn falls_back_to_the_sender_address() {
        let client = generate_keypair();
        let (firewall, seen) = recording();
        let mut relay = Relay::new(Some(firewall));
        let msg = client_message(&relay.server, &client);

        relay.respond_to(&msg, None);

        let seen = seen.lock().expect("seen");
        assert_eq!(
            *seen,
            vec![(client.public_key, SENDER_HOST.to_string(), SENDER_PORT)]
        );
    }
}

mod over_the_network {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn unlisted_peer_never_pairs() {
        // given: a relay that admits only A; A and B ask for the same token
        let n = gated_net(false).await;
        let token = rand::random::<[u8; 32]>();

        // when: both try to pair
        let (a, b) = tokio::join!(
            pair(&n, Who::A, token, true, 0x1001),
            pair(&n, Who::B, token, false, 0x2001)
        );

        // then: B never gets through, so nothing is matched
        assert!(b.is_err(), "B is refused, so its pairing fails");
        assert!(a.is_err(), "A has no partner");
        assert_eq!(n.relay.stats().pairings_matched, 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn listed_pair_matches() {
        // given: a relay that admits A and B
        let n = gated_net(true).await;
        let token = rand::random::<[u8; 32]>();

        // when: they pair on one token
        let (a, b) = tokio::join!(
            pair(&n, Who::A, token, true, 0x1001),
            pair(&n, Who::B, token, false, 0x2001)
        );

        // then: the relay matched them
        assert!(a.is_ok() && b.is_ok(), "both listed peers pair");
        assert!(n.relay.stats().pairings_matched >= 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn one_key_two_sessions_both_pair() {
        // given: A and B each hold two sessions on the same relay under one key
        let n = gated_net(true).await;
        let (t1, t2) = (rand::random::<[u8; 32]>(), rand::random::<[u8; 32]>());

        // when: both pairs form in parallel
        let (a1, b1, a2, b2) = tokio::join!(
            pair(&n, Who::A, t1, true, 0x1001),
            pair(&n, Who::B, t1, false, 0x2001),
            pair(&n, Who::A, t2, true, 0x1002),
            pair(&n, Who::B, t2, false, 0x2002)
        );

        // then: both pairs are matched
        assert!(a1.is_ok() && b1.is_ok(), "first pair");
        assert!(a2.is_ok() && b2.is_ok(), "second pair");
        assert!(n.relay.stats().pairings_matched >= 2);
    }
}

const SENDER_HOST: &str = "10.0.0.1";
const SENDER_PORT: u16 = 4000;

/// A relay's handshake path: its key, its repeat memory, and the admission callback.
struct Relay {
    server: KeyPair,
    dedup: Dedup,
    firewall: Option<Firewall>,
}

impl Relay {
    fn new(firewall: Option<Firewall>) -> Self {
        Self {
            server: KeyPair::generate(),
            dedup: Dedup::default(),
            firewall,
        }
    }

    fn handshake(&mut self, client: &NoiseKeypair) -> Response {
        let msg = client_message(&self.server, client);
        self.respond_to(&msg, None)
    }

    fn respond_to(&mut self, msg: &HandshakeMessage, peer_address: Option<Ipv4Peer>) -> Response {
        let gate = Gate {
            key_pair: &self.server,
            firewall: 0,
            peer_firewall: self.firewall.as_ref(),
        };
        let from = Ipv4Peer {
            host: SENDER_HOST.into(),
            port: SENDER_PORT,
        };
        respond(
            &mut self.dedup,
            &gate,
            msg,
            &from,
            peer_address,
            vec![],
            Instant::now(),
        )
    }
}

/// The callback that refuses every key outside `keys` (`true` = refuse).
fn only(keys: &[[u8; 32]]) -> Firewall {
    let keys = keys.to_vec();
    Arc::new(move |input| !keys.contains(input.remote_public_key))
}

type Seen = Arc<Mutex<Vec<([u8; 32], String, u16)>>>;

/// A callback that admits everyone and records (client key, client host, client port).
fn recording() -> (Firewall, Seen) {
    let seen = Seen::default();
    let sink = Arc::clone(&seen);
    let firewall: Firewall = Arc::new(move |input: &FirewallInput<'_>| {
        let at = input.client_address;
        sink.lock()
            .expect("sink")
            .push((*input.remote_public_key, at.host.clone(), at.port));
        false
    });
    (firewall, seen)
}

fn client_message(server: &KeyPair, client: &NoiseKeypair) -> HandshakeMessage {
    let mut noise = NoiseWrap::new_initiator(client.clone(), server.public_key);
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
        noise: noise.send(&payload).expect("client noise"),
        peer_address: None,
        relay_address: None,
    }
}

#[derive(Clone, Copy)]
enum Who {
    A,
    B,
}

/// A relay node that admits A (and B when `allow_b`) by their peer keys.
async fn gated_net(allow_b: bool) -> RelayNet {
    // The peers' keys exist only after the net is up, so the callback reads a list filled afterwards.
    let allowed = Arc::new(Mutex::new(Vec::<[u8; 32]>::new()));
    let read = Arc::clone(&allowed);
    let peer_firewall: Firewall = Arc::new(move |input| {
        !read
            .lock()
            .expect("allowed")
            .contains(input.remote_public_key)
    });
    let config = RelayServiceConfig {
        peer_firewall: Some(peer_firewall),
        ..RelayServiceConfig::default()
    };
    let n = relay_net(config).await;
    let mut allowed = allowed.lock().expect("allowed");
    allowed.push(n.a.key_pair.public_key);
    if allow_b {
        allowed.push(n.b.key_pair.public_key);
    }
    drop(allowed);
    n
}

/// Pair one token through the relay; gives up after 20 s.
async fn pair(
    n: &RelayNet,
    who: Who,
    token: [u8; 32],
    initiator: bool,
    id: u32,
) -> Result<RelayPaired, HyperDhtError> {
    let p = match who {
        Who::A => &n.a,
        Who::B => &n.b,
    };
    let paired = pair_via_relay(
        &p.handle,
        &p.key_pair,
        &p.rt,
        n.relay_pk,
        &[],
        token,
        initiator,
        id,
        Duration::from_secs(5),
    );
    tokio::time::timeout(Duration::from_secs(20), paired)
        .await
        .unwrap_or(Err(HyperDhtError::RelayAborted))
}
