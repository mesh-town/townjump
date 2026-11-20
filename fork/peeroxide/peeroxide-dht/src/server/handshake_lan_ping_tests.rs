//! Wiring of the same-host rule into the LAN probe: the handshake hands the client's hello to the
//! puzzle only when the client sits behind the server's own public address. Observed through the
//! server's debug line `server: lan ping sent`, which names its targets as `host:port`. The line
//! is written on whichever thread runs the server, so the process-wide log of
//! `punch_lan_ping_tests` is used, and a line is told from other tests' lines by the client's own
//! socket port. Included from `handshake_same_host_tests.rs`.
//!
//! A connect can finish before the server has seen a punch round (then it writes no line at all),
//! so each test makes several connects, each from a fresh client node, and judges them together.

use super::*;
use crate::hyperdht::LanRewrite;
use crate::server::punch_lan::punch_lan_ping_tests::capture;

/// Connects made per test.
const ATTEMPTS: usize = 5;

/// One client connects by hole-punch only (its own LAN candidate is a silent socket, so the LAN
/// path cannot win first): the number of server lines that probe this client's LAN address.
async fn probes_of_one_connect(client: &Node, count: &impl Fn(&str) -> usize) -> usize {
    let silent = std::net::UdpSocket::bind("127.0.0.1:0").expect("bind");
    let silent_port = silent.local_addr().expect("addr").port();
    let rewrite = move |_: &[Ipv4Peer]| vec![peer("127.0.0.1", silent_port)];
    let opts = ConnectOpts {
        lan_rewrite: Some(LanRewrite(std::sync::Arc::new(rewrite))),
        ..ConnectOpts::default()
    };
    let conn = (client.handle)
        .connect_with_options(&key(2), key(1).public_key, &[], &client.rt, opts)
        .await;
    let conn = conn.expect("connects by hole-punch");
    let port = conn.socket.local_addr().await.expect("socket").port();
    let needle = format!("{}:{port}", lan_host());
    let end = std::time::Instant::now() + Duration::from_secs(1);
    while count(&needle) == 0 && std::time::Instant::now() < end {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    count(&needle)
}

/// Probe lines per connect of [`ATTEMPTS`] clients; the first client is the fixture's.
async fn probes_per_connect(fx: &Fixture, count: &impl Fn(&str) -> usize) -> Vec<usize> {
    let mut all = vec![probes_of_one_connect(&fx.client, count).await];
    for _ in 1..ATTEMPTS {
        let client = match fx.client_remote.host != "127.0.0.1" {
            true => node_on(&fx.cl, &lan_host(), true).await,
            false => node(&fx.cl, true).await,
        };
        all.push(probes_of_one_connect(&client, count).await);
    }
    all
}

mod client_behind_the_server_public_address {
    use super::*;

    #[tokio::test]
    async fn server_pings_the_client_lan() {
        within(async {
            // given: the client shares the server's public address (127.0.0.1) and has a LAN address
            let count = capture::start();
            let fx = setup(Opts::same_host()).await;
            // when: clients connect by hole-punch
            let probes = probes_per_connect(&fx, &count).await;
            // then: the server pinged the LAN address of at least one of them
            assert!(probes.iter().any(|&n| n > 0), "no LAN probe: {probes:?}");
        })
        .await;
    }
}

mod client_from_another_host_no_ping {
    use super::*;

    #[tokio::test]
    async fn server_does_not_ping_the_client_lan() {
        within(async {
            // given: the clients are bound to the LAN address: another host than the server's
            let count = capture::start();
            let fx = setup(Opts::other_host()).await;
            // when: clients connect by hole-punch
            let probes = probes_per_connect(&fx, &count).await;
            // then: the server pinged none of them
            assert!(probes.iter().all(|&n| n == 0), "LAN probes: {probes:?}");
        })
        .await;
    }
}
