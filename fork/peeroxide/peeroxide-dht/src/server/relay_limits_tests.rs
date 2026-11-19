//! Cluster test: the node-wide `StreamLimits` reach the stream of a relayed connection on both sides
//! (client: `over_relay`, server: `serve_relayed`).

use std::time::Duration;

use crate::hyperdht::{ConnectOpts, HolepunchFilter, RelayThrough, StreamLimits};
use crate::server::ServerOpts;
use crate::server::relay_flow_tests::relay;
use crate::server::test_net::{cluster, key, node_limited, serve, within};

const CLIENT_RECV: u32 = 131_072;
const SERVER_RECV: u32 = 262_144;

#[tokio::test(flavor = "multi_thread")]
async fn limits_apply_to_both_ends_of_a_relayed_connection() {
    within(async {
        // given: a cluster with a live relay, a firewalled server and a firewalled client, each node with its own limits
        let cl = cluster(4).await;
        let server_limits = StreamLimits {
            recv_window_max: Some(SERVER_RECV),
            ..StreamLimits::default()
        };
        let client_limits = StreamLimits {
            recv_window_max: Some(CLIENT_RECV),
            ..StreamLimits::default()
        };
        let opts = ServerOpts {
            holepunch: Some(HolepunchFilter::new(|_| false)),
            ..ServerOpts::default()
        };
        let mut served = serve(node_limited(&cl, true, server_limits).await, &key(1), opts).await;
        let live = relay(&cl).await;
        let client = node_limited(&cl, true, client_limits).await;

        // when: the client connects with the direct path closed, so the stream runs over the relay
        let connect_opts = ConnectOpts {
            local_connection: false,
            relay_through: Some(RelayThrough::Key(live.pk)),
            holepunch: Some(HolepunchFilter::new(|_| false)),
            relay_start: Some(0),
            ..ConnectOpts::default()
        };
        let conn = client
            .handle
            .connect_with_options(&key(2), key(1).public_key, &[], &client.rt, connect_opts)
            .await
            .expect("connected over the relay");
        let accepted = tokio::time::timeout(Duration::from_secs(20), served.incoming.recv())
            .await
            .expect("server accepted in time")
            .expect("incoming connection");

        // then: the relay really paired them, and each end shows the limit of its own node
        assert!(live.stats.stats().pairings_matched >= 1, "no relay pairing");
        assert_eq!(conn.stream.get_ref().recv_window_max(), CLIENT_RECV);
        assert_eq!(accepted.stream.get_ref().recv_window_max(), SERVER_RECV);
    })
    .await;
}
