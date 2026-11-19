//! Cluster test of the failing arm of `connect_relayed` (client race between hole punching and a blind-relay pairing):
//! when the relay pairing fails and the punch has nothing left to offer, the connect returns the error instead of waiting on.
//!
//! The test sits next to the server cluster tests because only they have the local network it needs.

use std::time::{Duration, Instant};

use crate::hyperdht::{ConnectOpts, HolepunchFilter, HyperDhtError, RelayThrough};
use crate::server::ServerOpts;
use crate::server::test_net::{cluster, key, node, serve, within};

#[tokio::test(flavor = "multi_thread")]
async fn relay_error_with_a_closed_punch_makes_connect_fail() {
    within(async {
        // given: a server that refuses punching, and a relay key that answers the handshake but speaks no blind-relay
        let cl = cluster(4).await;
        let opts = ServerOpts {
            holepunch: Some(HolepunchFilter::new(|_| false)),
            ..ServerOpts::default()
        };
        let _server = serve(node(&cl, true).await, &key(1), opts).await;
        let _dead = serve(node(&cl, false).await, &key(9), ServerOpts::default()).await;
        let client = node(&cl, true).await;
        let connect_opts = ConnectOpts {
            local_connection: false,
            relay_through: Some(RelayThrough::Key(key(9).public_key)),
            holepunch: Some(HolepunchFilter::new(|_| false)),
            relay_start: Some(0),
            ..ConnectOpts::default()
        };

        // when: the client connects, so the pairing fails after its own limit and the punch has already failed
        let started = Instant::now();
        let out = client
            .handle
            .connect_with_options(&key(2), key(1).public_key, &[], &client.rt, connect_opts)
            .await;

        // then: the connect ends with `RelayAborted`, and only after the pairing was really tried
        assert!(
            matches!(out, Err(HyperDhtError::RelayAborted)),
            "{:?}",
            out.err()
        );
        assert!(
            started.elapsed() >= Duration::from_secs(10),
            "failed before the pairing ran its course: {:?}",
            started.elapsed()
        );
    })
    .await;
}
