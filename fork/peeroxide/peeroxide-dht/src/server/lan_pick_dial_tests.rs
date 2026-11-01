//! `connect_to` keeps the address that answered as the dial target, even when the server
//! announces a public address first; `find_and_connect` to the same server does not. The server's
//! `addresses4` get a public address (nobody listens there) through a test lever.

use std::time::Duration;

use super::ServerOpts;
use super::reply_build::inject_addresses4;
use super::test_net::{cluster, key, node, serve, within};
use crate::hyperdht::KeyPair;
use crate::messages::Ipv4Peer;

const SILENT_PUBLIC: &str = "8.8.8.8";

#[tokio::test]
async fn connect_to_dials_the_address_that_answered() {
    within(async {
        // given: an open server announcing an unreachable public address first
        let cl = cluster(3).await;
        let server = serve(node(&cl, false).await, &key(0x51), ServerOpts::default()).await;
        let port = server.handle.local_port().await.expect("server port");
        inject_addresses4(
            port,
            vec![Ipv4Peer {
                host: SILENT_PUBLIC.into(),
                port: 9,
            }],
        );
        let client = node(&cl, false).await;
        let me = KeyPair::from_seed([0x52; 32]);
        let server_addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));

        // when: the client dials the server's address directly
        let conn = tokio::time::timeout(
            Duration::from_secs(15),
            client
                .handle
                .connect_to(&me, key(0x51).public_key, server_addr, &client.rt),
        )
        .await
        .expect("connect_to timed out: it dialed the announced address")
        .expect("connect_to");

        // then: the dialed address is the one that answered
        assert_eq!(conn.remote_addr, Some(server_addr));

        // and: a lookup-based connect to the same server does not take that address
        let found = tokio::time::timeout(
            Duration::from_secs(5),
            client.handle.connect(&me, key(0x51).public_key, &client.rt),
        )
        .await;
        if let Ok(Ok(conn)) = found {
            assert_ne!(
                conn.remote_addr,
                Some(server_addr),
                "find_and_connect took the server address"
            );
        }
    })
    .await;
}
