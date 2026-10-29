//! Tests for [`crate::server::relay`].

use super::*;
use crate::hyperdht::RelayThrough;
use crate::hyperdht_messages::{FIREWALL_UNKNOWN, RelayThroughInfo as Info};
use crate::server::ServerOpts;
use crate::server::test_net::{
    Cluster, Node, cluster, hello, key, node, raw_handshake, serve, within,
};

fn info(tag: u8) -> Info {
    Info {
        version: 1,
        public_key: [tag; 32],
        token: [tag + 100; 32],
    }
}

#[test]
fn choose_roles() {
    // given: our relay and the client's relay
    let own = info(1);
    let remote = info(2);
    // when/then: ours wins and we initiate with our token
    let plan = choose(Some(&own), Some(&remote)).expect("plan");
    assert_eq!(
        (plan.relay_pk, plan.token, plan.is_initiator),
        ([1; 32], [101; 32], true)
    );
    let plan = choose(Some(&own), None).expect("plan");
    assert_eq!((plan.relay_pk, plan.is_initiator), ([1; 32], true));
    // when/then: only the client's relay: we join without initiating, with its token
    let plan = choose(None, Some(&remote)).expect("plan");
    assert_eq!(
        (plan.relay_pk, plan.token, plan.is_initiator),
        ([2; 32], [102; 32], false)
    );
    // when/then: nothing, or an empty list
    assert_eq!(choose(None, None), None);
    let opts = ServerOpts {
        relay_through: Some(RelayThrough::List(vec![])),
        ..ServerOpts::default()
    };
    assert_eq!(own_relay(&opts, None), None);
    assert_eq!(own_relay(&ServerOpts::default(), None), None);
}

#[test]
fn open_clients_get_no_relay() {
    assert!(!relay_allowed(Plan::DialClient));
    assert!(relay_allowed(Plan::AwaitClient) && relay_allowed(Plan::Holepunch));
}

/// The reply bytes of one raw handshake through a cluster node.
async fn shake(cl: &Cluster, client: &Node, noise: &[u8]) -> Vec<u8> {
    let res = raw_handshake(client, key(1).public_key, noise, &cl.addrs[0]).await;
    res.expect("server replies").noise
}

#[tokio::test(flavor = "multi_thread")]
async fn server_announces_relay_through_with_fresh_token() {
    within(async {
        // given: a firewalled server announcing relay R, and a raw client
        let cl = cluster(3).await;
        let opts = ServerOpts {
            relay_through: Some(RelayThrough::Key([9; 32])),
            ..ServerOpts::default()
        };
        let _served = serve(node(&cl, true).await, &key(1), opts).await;
        let client = node(&cl, true).await;
        let spk = key(1).public_key;
        // when: two clients shake hands, the first one's message is sent twice
        let (mut nw_a, noise_a) = hello(&key(2), spk, FIREWALL_UNKNOWN, Some(41));
        let (mut nw_b, noise_b) = hello(&key(3), spk, FIREWALL_UNKNOWN, Some(42));
        let reply_a = shake(&cl, &client, &noise_a).await;
        let reply_b = shake(&cl, &client, &noise_b).await;
        let repeat_a = shake(&cl, &client, &noise_a).await;
        // then: version 1, relay R, a fresh token per client, the same reply on a repeat
        let ra = nw_a
            .recv(&reply_a)
            .expect("payload")
            .relay_through
            .expect("announced");
        assert_eq!((ra.version, ra.public_key), (1, [9; 32]));
        let rb = nw_b
            .recv(&reply_b)
            .expect("payload")
            .relay_through
            .expect("announced");
        assert_ne!(ra.token, rb.token);
        assert_eq!(repeat_a, reply_a);
    })
    .await;
}
