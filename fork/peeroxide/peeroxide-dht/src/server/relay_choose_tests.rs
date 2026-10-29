//! Which relay the server announces and who initiates the pair (a server with its own `List`
//! yields to a relay of that list the client asks for), and the key the server uses at its relay.
// JS: hyperdht/lib/server.js:412-421

use std::any::Any;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::{RelayPlan, choose, own_relay};
use crate::crypto::hash;
use crate::hyperdht::{ConnectOpts, HolepunchFilter, KeyPair, RelayThrough};
use crate::hyperdht_messages::RelayThroughInfo as Info;
use crate::relay_service::{RelayServiceConfig, run_relay_server};
use crate::server::relay_flow_tests::dial;
use crate::server::test_net::{
    Cluster, Node, cluster, key, lan_host, member_on, node, serve, within,
};
use crate::server::{Firewall, FirewallInput, ServerOpts};

mod server_with_own_list {
    use super::*;

    mod client_asks_for_a_key_of_that_list {
        use super::*;

        #[test]
        fn answer_carries_no_relay_through() {
            // given: own List[a, b], the client asks for b
            let opts = with_relay(RelayThrough::List(vec![[1; 32], [2; 32]]));
            // when: the server decides
            let (own, _) = decide(&opts, Some(&client_info(2)));
            // then: it announces no relay of its own
            assert_eq!(own, None);
        }

        #[test]
        fn server_joins_the_clients_relay_without_initiating() {
            let opts = with_relay(RelayThrough::List(vec![[1; 32], [2; 32]]));
            let client = client_info(2);
            let (_, plan) = decide(&opts, Some(&client));
            assert_eq!(
                plan,
                Some(RelayPlan {
                    relay_pk: [2; 32],
                    token: client.token,
                    is_initiator: false
                })
            );
        }
    }

    mod client_asks_for_a_key_outside_the_list {
        use super::*;

        #[test]
        fn server_announces_a_relay_from_its_list() {
            // given: own List[a, b], the client asks for c
            let opts = with_relay(RelayThrough::List(vec![[1; 32], [2; 32]]));
            // when: the server decides
            let (own, _) = decide(&opts, Some(&client_info(3)));
            // then: the announced relay is one of the list
            let own = own.expect("a relay of its own");
            assert!(
                [[1; 32], [2; 32]].contains(&own.public_key),
                "relay {:?}",
                own.public_key
            );
        }

        #[test]
        fn server_initiates_with_its_own_token() {
            let opts = with_relay(RelayThrough::List(vec![[1; 32], [2; 32]]));
            let client = client_info(3);
            let (own, plan) = decide(&opts, Some(&client));
            let (own, plan) = (own.expect("own relay"), plan.expect("plan"));
            assert!(plan.is_initiator, "the server must initiate");
            assert_eq!((plan.relay_pk, plan.token), (own.public_key, own.token));
            assert_ne!(
                plan.token, client.token,
                "the client's token must not be used"
            );
        }
    }

    mod client_asks_for_no_relay {
        use super::*;

        #[test]
        fn server_initiates_through_its_list() {
            let opts = with_relay(RelayThrough::List(vec![[1; 32], [2; 32]]));
            let (own, plan) = decide(&opts, None);
            let plan = plan.expect("plan");
            assert!(own.is_some() && plan.is_initiator);
            assert!([[1; 32], [2; 32]].contains(&plan.relay_pk));
        }
    }
}

mod server_with_own_key {
    use super::*;

    #[test]
    fn same_key_asked_by_the_client_is_still_the_servers_and_it_initiates() {
        // given: own Key(a), the client asks for a as well
        let opts = with_relay(RelayThrough::Key([1; 32]));
        let client = client_info(1);
        // when: the server decides
        let (own, plan) = decide(&opts, Some(&client));
        // then: it announces a (with its own token) and initiates
        let (own, plan) = (own.expect("own relay"), plan.expect("plan"));
        assert_eq!(own.public_key, [1; 32]);
        assert!(plan.is_initiator, "the server must initiate");
        assert_eq!((plan.relay_pk, plan.token), ([1; 32], own.token));
        assert_ne!(
            plan.token, client.token,
            "the client's token must not be used"
        );
    }
}

mod relay_key_pair {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn relay_sees_the_given_key_not_the_server_identity() {
        within(async {
            // given: a relay that records the keys of its peers, a server with relay_key_pair
            let cl = cluster(4).await;
            let relay = recording_relay(&cl).await;
            let actor = key(7);
            let opts = ServerOpts {
                relay_through: Some(RelayThrough::Key(relay.pk)),
                relay_key_pair: Some(actor.clone()),
                ..ServerOpts::default()
            };
            let _served = serve(node(&cl, true).await, &key(1), opts).await;
            // when: a client connects and the server pairs at the relay
            let _conn = dial(&cl, closed_punch(), 2).await;
            // then: the relay saw the given key, and not the identity the server listens on
            let seen = relay.seen();
            assert!(seen.contains(&actor.public_key), "relay saw only {seen:?}");
            assert!(
                !seen.contains(&key(1).public_key),
                "the server identity leaked to the relay"
            );
        })
        .await;
    }
}

fn client_info(tag: u8) -> Info {
    Info {
        version: 1,
        public_key: [tag; 32],
        token: [tag + 100; 32],
    }
}

fn with_relay(relay_through: RelayThrough) -> ServerOpts {
    ServerOpts {
        relay_through: Some(relay_through),
        ..ServerOpts::default()
    }
}

/// What the handshake does with the client's request: the relay the server announces, the plan.
fn decide(opts: &ServerOpts, client: Option<&Info>) -> (Option<Info>, Option<RelayPlan>) {
    let own = own_relay(opts, client);
    let plan = choose(own.as_ref(), client);
    (own, plan)
}

fn closed_punch() -> ConnectOpts {
    ConnectOpts {
        local_connection: false,
        holepunch: Some(HolepunchFilter::new(|_| false)),
        ..ConnectOpts::default()
    }
}

/// A blind-relay node that admits everyone and records the key of every peer it is shaken by.
struct Recording {
    pk: [u8; 32],
    keys: Arc<Mutex<Vec<[u8; 32]>>>,
    _node: Node,
    _service: Box<dyn Any + Send>,
}

impl Recording {
    fn seen(&self) -> Vec<[u8; 32]> {
        self.keys.lock().expect("keys").clone()
    }
}

async fn recording_relay(cl: &Cluster) -> Recording {
    let mut relay_node = member_on(cl, &lan_host()).await;
    let kp = KeyPair::generate();
    let events = std::mem::replace(
        &mut relay_node.events,
        tokio::sync::mpsc::unbounded_channel().1,
    );
    let keys = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&keys);
    let record: Firewall = Arc::new(move |input: &FirewallInput<'_>| {
        log.lock().expect("keys").push(*input.remote_public_key);
        false
    });
    let config = RelayServiceConfig {
        peer_firewall: Some(record),
        ..Default::default()
    };
    let (_stats, service) = run_relay_server(
        relay_node.rt.handle(),
        relay_node.handle.clone(),
        kp.clone(),
        events,
        config,
    );
    for _ in 0..100 {
        let found = cl.nodes[0].handle.find_peer(hash(&kp.public_key)).await;
        if matches!(found, Ok(Some(_))) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    Recording {
        pk: kp.public_key,
        keys,
        _node: relay_node,
        _service: Box::new(service),
    }
}
