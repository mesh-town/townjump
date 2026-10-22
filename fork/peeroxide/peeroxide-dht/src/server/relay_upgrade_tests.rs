//! Cluster test of the server's side of the relay-to-direct upgrade: the first packet from the
//! client's new address moves the server's path from the relay to a direct one.

use std::time::Duration;

use tokio::sync::{oneshot, watch};

use super::ServerOpts;
use super::relay_flow_tests::{dial, relay};
use crate::hyperdht::{ConnectOpts, PeerConnection, RelayPath, RelayThrough};
use crate::server::test_net::{Served, cluster, key, node, serve, sha256, within};

const CHUNK: usize = 64 * 1024;

/// Accepts one connection, hands its path to the test, echoes until the end.
fn echo_with_path(served: Served) -> oneshot::Receiver<watch::Receiver<RelayPath>> {
    let (tx, rx) = oneshot::channel();
    tokio::spawn(async move {
        let mut served = served;
        let Some(mut conn) = served.incoming.recv().await else {
            return;
        };
        if let Some(path) = conn.relay_path() {
            let _ = tx.send(path);
        }
        while let Ok(Some(data)) = conn.stream.read().await {
            if conn.stream.write(&data).await.is_err() {
                return;
            }
        }
    });
    rx
}

/// Writes 256 KiB in chunks, reading each chunk back before the next; returns sha256 of the echo.
async fn echo_256k(conn: &mut PeerConnection, seed: usize) -> ([u8; 32], [u8; 32]) {
    let data: Vec<u8> = (0..4 * CHUNK).map(|i| ((i + seed) % 251) as u8).collect();
    let mut back = Vec::new();
    for chunk in data.chunks(CHUNK) {
        conn.stream.write(chunk).await.expect("client write");
        let mut got = 0;
        while got < chunk.len() {
            let part = conn.stream.read().await.expect("read").expect("not closed");
            got += part.len();
            back.extend_from_slice(&part);
        }
    }
    (sha256(&data), sha256(&back))
}

#[tokio::test(flavor = "multi_thread")]
async fn server_upgrades_on_first_packet_from_new_address() {
    within(async {
        // given: both sides firewalled, the server announces R, the client waits 2 s before punching
        let cl = cluster(4).await;
        let r = relay(&cl).await;
        let opts = ServerOpts {
            relay_through: Some(RelayThrough::Key(r.pk)),
            ..ServerOpts::default()
        };
        let path = echo_with_path(serve(node(&cl, true).await, &key(1), opts).await);
        let client = ConnectOpts {
            local_connection: false,
            punch_delay: Some(Duration::from_secs(2)),
            ..ConnectOpts::default()
        };
        let mut conn = dial(&cl, client, 2).await;
        // when: 256 KiB go through the relay
        let mut path = path.await.expect("server connection has a relay link");
        assert_eq!(*path.borrow(), RelayPath::Relayed);
        let (sent, got) = echo_256k(&mut conn, 0).await;
        assert_eq!(sent, got, "echo before the upgrade");
        // then: within 15 s the server's path is direct and the data still flows
        let direct = path.wait_for(|p| *p == RelayPath::Direct);
        let direct = tokio::time::timeout(Duration::from_secs(15), direct).await;
        assert!(direct.is_ok(), "server path became direct in 15 s");
        let (sent, got) = echo_256k(&mut conn, 7).await;
        assert_eq!(sent, got, "echo after the upgrade");
        // then: R holds no pairing any more
        for _ in 0..50 {
            if r.stats.stats().pairings_active == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        assert_eq!(r.stats.stats().pairings_active, 0);
    })
    .await;
}
