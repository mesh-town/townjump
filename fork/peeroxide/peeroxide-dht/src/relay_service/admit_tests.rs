//! Admission of the relay service over the network: `peer_firewall` refuses silently (no session, no
//! pairing), a listed key is served, one key may hold several sessions, and the callback is told who
//! asks and from where.
// JS: blind-relay/index.js

use std::sync::{Arc, Mutex};

use crate::relay_service::RelayServiceConfig;
use crate::relay_service::test_net::{RelayNet, Who, pair, relay_net};
use crate::server::{Firewall, FirewallInput};

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

    #[tokio::test(flavor = "multi_thread")]
    async fn callback_sees_the_client_key_and_address() {
        // given: a relay whose callback admits everyone and records (key, client address)
        let (firewall, seen) = recording();
        let n = relay_net(RelayServiceConfig {
            peer_firewall: Some(firewall),
            ..RelayServiceConfig::default()
        })
        .await;
        let token = rand::random::<[u8; 32]>();

        // when: A (and its partner B) pair through the relay
        let (a, b) = tokio::join!(
            pair(&n, Who::A, token, true, 0x1001),
            pair(&n, Who::B, token, false, 0x2001)
        );
        assert!(a.is_ok() && b.is_ok(), "both peers pair");

        // then: the callback saw A's key with A's loopback address and a real port
        let seen = seen.lock().expect("seen");
        let record = seen
            .iter()
            .find(|(key, _, _)| *key == n.a.key_pair.public_key)
            .expect("the callback was called for A");
        assert_eq!(record.1, "127.0.0.1", "client host");
        assert_ne!(record.2, 0, "client port");
    }
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
