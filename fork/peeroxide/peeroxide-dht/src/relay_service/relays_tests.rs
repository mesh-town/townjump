//! The relays the control-connection server advertises after the relay's self-announce: every node
//! that accepted the record, not only the first few. A punch round is verified only through one of them.
// JS: hyperdht/lib/server.js

use std::time::{Duration, Instant};

use crate::relay_service::RelayServiceConfig;
use crate::relay_service::test_net::relay_net;

#[tokio::test(flavor = "multi_thread")]
async fn server_relays_hold_every_node_that_accepted_the_announce() {
    // given: a relay announced to a network with four routable holders
    let n = relay_net(RelayServiceConfig::default()).await;
    let service = n.service.as_ref().expect("service handle");

    // when: the first announce has handed its relays to the server
    let start = Instant::now();
    let mut relays = service.server_relays().await;
    while relays.is_empty() {
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "the server never got relays from the announce"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
        relays = service.server_relays().await;
    }

    // then: all four holders are there, not just the first three
    let mut hosts: Vec<_> = relays.iter().map(|r| r.relay_address.port).collect();
    hosts.sort_unstable();
    hosts.dedup();
    assert_eq!(
        relays.len(),
        4,
        "every accepting node is a relay of the server: {relays:?}"
    );
    assert_eq!(hosts.len(), 4, "the relays are distinct nodes");
    n.service.expect("service handle").close().await;
}
