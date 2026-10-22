//! Client `relay_through` on a local cluster: the hole punch races a blind-relay pairing.
//! "Punching impossible" = a server that must be punched to, a client with `local_connection`
//! off and a `holepunch` filter that says no. 127.0.0.1 only.
mod common;

use std::time::{Duration, Instant};

use common::{
    Relay, Served, cluster, serve_key, serve_key_fw, serve_key_opts, spawn_node_fw, spawn_relay,
    within,
};
use peeroxide_dht::crypto::hash;
use peeroxide_dht::hyperdht::{
    ConnectOpts, HolepunchFilter, HyperDhtError, KeyPair, PeerConnection, RelayPath, RelayThrough,
};
use peeroxide_dht::server::ServerOpts;
use sha2::{Digest, Sha256};

fn opts(relay: Option<[u8; 32]>, punch_allowed: bool) -> ConnectOpts {
    let mut o = ConnectOpts::default();
    o.local_connection = false;
    o.relay_through = relay.map(RelayThrough::Key);
    o.holepunch = Some(HolepunchFilter::new(move |_| punch_allowed));
    o
}

/// The relay key serves, but not blind-relay: pairing never completes, the punch is vetoed.
#[tokio::test]
async fn relay_never_pairs_aborts() {
    within(async {
        let (_nodes, bootstrap) = cluster(4).await;
        let key = KeyPair::from_seed([0x81; 32]);
        let _served = serve_key_fw(&bootstrap, &key, false, true).await;
        let plain = KeyPair::from_seed([0x82; 32]);
        let _plain = serve_key(&bootstrap, &plain, false).await;
        let client = spawn_node_fw(bootstrap, true, true).await;
        let me = KeyPair::from_seed([0x83; 32]);

        let started = Instant::now();
        let r = client
            .handle
            .connect_with_options(
                &me,
                key.public_key,
                &[],
                &client.rt,
                opts(Some(plain.public_key), false),
            )
            .await;
        let elapsed = started.elapsed();

        assert!(
            matches!(r, Err(HyperDhtError::RelayAborted)),
            "{:?}",
            r.err()
        );
        assert!(elapsed >= Duration::from_secs(10), "{elapsed:?}");
        assert!(elapsed < Duration::from_secs(20), "{elapsed:?}");
    })
    .await;
}

/// A relay nobody announced fails fast; the punch carries the connection, no relay path.
#[tokio::test]
async fn relay_down_punch_ok_goes_direct() {
    within(async {
        let (_nodes, bootstrap) = cluster(4).await;
        let key = KeyPair::from_seed([0x84; 32]);
        let mut served = serve_key_fw(&bootstrap, &key, false, true).await;
        let client = spawn_node_fw(bootstrap, true, true).await;
        let me = KeyPair::from_seed([0x85; 32]);
        let mut o = opts(Some([0x86; 32]), true);
        o.holepunch = None;

        let conn = client
            .handle
            .connect_with_options(&me, key.public_key, &[], &client.rt, o)
            .await
            .expect("the punch wins");

        assert!(conn.relay_path().is_none());
        assert!(served.incoming.recv().await.is_some());
    })
    .await;
}

/// Without a relay the filter's veto surfaces as the punch error.
#[tokio::test]
async fn holepunch_filter_without_relay_aborts() {
    within(async {
        let (_nodes, bootstrap) = cluster(4).await;
        let key = KeyPair::from_seed([0x87; 32]);
        let _served = serve_key_fw(&bootstrap, &key, false, true).await;
        let client = spawn_node_fw(bootstrap, true, true).await;
        let me = KeyPair::from_seed([0x88; 32]);

        let r = client
            .handle
            .connect_with_options(&me, key.public_key, &[], &client.rt, opts(None, false))
            .await;

        assert!(
            matches!(r, Err(HyperDhtError::HolepunchAborted)),
            "{:?}",
            r.err()
        );
    })
    .await;
}

fn path(conn: &PeerConnection) -> Option<RelayPath> {
    conn.relay_path().map(|rx| *rx.borrow())
}

/// A relay that the whole cluster can already find by its key.
async fn announced_relay(bootstrap: &[String]) -> Relay {
    let relay = spawn_relay(bootstrap).await;
    let probe = spawn_node_fw(bootstrap.to_vec(), true, false).await;
    for _ in 0..50 {
        let found = probe.handle.find_peer(hash(&relay.public_key)).await;
        if matches!(found, Ok(Some(_))) {
            return relay;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("relay was never announced");
}

/// Accepts one connection and echoes it until the client shuts down.
fn echo_one(mut served: Served) {
    tokio::spawn(async move {
        let Some(mut conn) = served.incoming.recv().await else {
            return;
        };
        while let Ok(Some(data)) = conn.stream.read().await {
            if conn.stream.write(&data).await.is_err() {
                return;
            }
        }
        let _ = conn.stream.shutdown().await;
        drop(served);
    });
}

/// Writes `len` bytes, shuts the write side down, reads the echo to the end; returns its sha256.
async fn echo_digest(conn: PeerConnection, len: usize) -> ([u8; 32], [u8; 32]) {
    let data: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
    let sent: [u8; 32] = Sha256::digest(&data).into();
    let (mut reader, mut writer) = conn.stream.into_split();
    let sender = tokio::spawn(async move {
        for chunk in data.chunks(16 * 1024) {
            writer.write(chunk).await.expect("client write");
        }
        writer.shutdown().await.expect("client shutdown");
    });
    let mut digest = Sha256::new();
    while let Some(piece) = reader.read().await.expect("client read") {
        digest.update(&piece);
    }
    sender.await.expect("sender");
    (sent, digest.finalize().into())
}

async fn dial(
    bootstrap: Vec<String>,
    server: [u8; 32],
    o: ConnectOpts,
    firewalled: bool,
) -> (common::Node, PeerConnection) {
    let client = spawn_node_fw(bootstrap, true, firewalled).await;
    let me = KeyPair::from_seed([0x90; 32]);
    let conn = client
        .handle
        .connect_with_options(&me, server, &[], &client.rt, o)
        .await
        .expect("connected");
    (client, conn)
}

/// Punching refused on the client side, the client names the relay: data flows over it.
#[tokio::test]
async fn client_relay_through_carries_data() {
    within(async {
        let (_nodes, bootstrap) = cluster(4).await;
        let relay = announced_relay(&bootstrap).await;
        let key = KeyPair::from_seed([0x91; 32]);
        echo_one(serve_key_fw(&bootstrap, &key, false, true).await);

        let o = opts(Some(relay.public_key), false);
        let (_client, conn) = dial(bootstrap, key.public_key, o, true).await;

        assert_eq!(path(&conn), Some(RelayPath::Relayed));
        let (sent, echoed) = echo_digest(conn, 64 * 1024).await;
        assert_eq!(sent, echoed);
        assert_eq!(relay.server.stats().pairings_matched, 1);
    })
    .await;
}

/// The server names the relay, the client does not: the client still ends up on it.
#[tokio::test]
async fn server_relay_through_used_by_plain_client() {
    within(async {
        let (_nodes, bootstrap) = cluster(4).await;
        let relay = announced_relay(&bootstrap).await;
        let key = KeyPair::from_seed([0x92; 32]);
        let mut sopts = ServerOpts::default();
        sopts.relay_through = Some(RelayThrough::Key(relay.public_key));
        echo_one(serve_key_opts(&bootstrap, &key, true, sopts).await);

        let (_client, conn) = dial(bootstrap, key.public_key, opts(None, false), true).await;

        assert_eq!(path(&conn), Some(RelayPath::Relayed));
        let (sent, echoed) = echo_digest(conn, 64 * 1024).await;
        assert_eq!(sent, echoed);
        assert_eq!(relay.server.stats().pairings_matched, 1);
    })
    .await;
}

/// A direct path exists: the connection stays direct, the relay pair is made in the background.
#[tokio::test]
async fn relay_through_with_direct_path_stays_direct() {
    within(async {
        let (_nodes, bootstrap) = cluster(4).await;
        let relay = announced_relay(&bootstrap).await;
        let key = KeyPair::from_seed([0x93; 32]);
        echo_one(serve_key(&bootstrap, &key, false).await);

        let mut o = ConnectOpts::default();
        o.relay_through = Some(RelayThrough::Key(relay.public_key));
        let started = Instant::now();
        let (_client, conn) = dial(bootstrap, key.public_key, o, false).await;
        assert!(started.elapsed() < Duration::from_secs(10));
        assert_eq!(path(&conn), None);

        // The client asks the relay in the background; the server's slot of the handshake that
        // did not carry the connection asks as well (its pair never completes). Two requests
        // in all, one of them the client's: without the background pair there is one.
        let mut stats = relay.server.stats();
        for _ in 0..25 {
            stats = relay.server.stats();
            if stats.pairings_requested >= 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        assert!(stats.pairings_requested >= 2, "{stats:?}");
        tokio::time::sleep(Duration::from_secs(3)).await;
        let (sent, echoed) = echo_digest(conn, 32 * 1024).await;
        assert_eq!(sent, echoed);
    })
    .await;
}
