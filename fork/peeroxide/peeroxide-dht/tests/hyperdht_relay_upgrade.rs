//! Relay-to-direct upgrade on a local cluster: the client punches 2 s after the relay path is up,
//! the connection moves to the direct path and the relay lets go of the pairing.
//! The relay listens on a LAN address (see `common::spawn_relay`), the peers on 127.0.0.1.
mod common;

use std::time::Duration;

use common::{Relay, Served, cluster, serve_key_opts, spawn_node_fw, spawn_relay, within};
use peeroxide_dht::crypto::hash;
use peeroxide_dht::hyperdht::{
    ConnectOpts, HolepunchFilter, KeyPair, PeerConnection, RelayPath, RelayThrough,
};
use peeroxide_dht::server::ServerOpts;
use sha2::{Digest, Sha256};

const CHUNK: usize = 16 * 1024;

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

/// Echoes until the client shuts down; both peers write, so neither side stays silent.
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

/// Writes `len` bytes in chunks, reading each chunk back before the next; (sent, echoed) sha256.
async fn echo_roundtrip(
    conn: &mut PeerConnection,
    len: usize,
    seed: usize,
) -> ([u8; 32], [u8; 32]) {
    let data: Vec<u8> = (0..len).map(|i| ((i + seed) % 251) as u8).collect();
    let mut back = Sha256::new();
    for chunk in data.chunks(CHUNK) {
        conn.stream.write(chunk).await.expect("client write");
        let mut got = 0;
        while got < chunk.len() {
            let part = conn.stream.read().await.expect("read").expect("not closed");
            got += part.len();
            back.update(&part);
        }
    }
    (Sha256::digest(&data).into(), back.finalize().into())
}

async fn dial(
    bootstrap: Vec<String>,
    server: [u8; 32],
    o: ConnectOpts,
) -> (common::Node, PeerConnection) {
    let client = spawn_node_fw(bootstrap, true, false).await;
    let me = KeyPair::from_seed([0x90; 32]);
    let conn = client
        .handle
        .connect_with_options(&me, server, &[], &client.rt, o)
        .await
        .expect("connected");
    (client, conn)
}

/// Waits (up to `secs`) until the relay holds no pairing and has closed every stream it opened.
async fn relay_let_go(relay: &Relay, secs: u64) -> bool {
    for _ in 0..secs * 5 {
        let s = relay.server.stats();
        if s.pairings_active == 0 && s.streams_opened == s.streams_closed && s.streams_closed >= 2 {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    false
}

/// Waits (up to `secs`) until every control session of the relay is closed: the winning pair and
/// the slots of handshakes that never opened.
async fn relay_sessions_closed(relay: &Relay, secs: u64) -> bool {
    for _ in 0..secs * 5 {
        if relay.server.stats().sessions_active == 0 {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    false
}

/// The server announces the relay, the client waits 2 s before it punches.
#[tokio::test(flavor = "multi_thread")]
async fn upgrade_without_loss() {
    within(async {
        let (_nodes, bootstrap) = cluster(4).await;
        let relay = announced_relay(&bootstrap).await;
        let key = KeyPair::from_seed([0x95; 32]);
        let mut sopts = ServerOpts::default();
        sopts.relay_through = Some(RelayThrough::Key(relay.public_key));
        echo_one(serve_key_opts(&bootstrap, &key, true, sopts).await);
        let mut o = ConnectOpts::default();
        o.local_connection = false;
        o.relay_through = Some(RelayThrough::Key(relay.public_key));
        o.punch_delay = Some(Duration::from_secs(2));
        let (_client, mut conn) = dial(bootstrap, key.public_key, o).await;

        let mut path = conn.relay_path().expect("relay link");
        assert_eq!(*path.borrow(), RelayPath::Relayed);
        let (sent, got) = echo_roundtrip(&mut conn, 512 * 1024, 0).await;
        assert_eq!(sent, got, "echo before the upgrade");

        let direct = path.wait_for(|p| *p == RelayPath::Direct);
        let direct = tokio::time::timeout(Duration::from_secs(15), direct).await;
        assert!(direct.is_ok(), "client path became direct in 15 s");
        let (sent, got) = echo_roundtrip(&mut conn, 512 * 1024, 7).await;
        assert_eq!(sent, got, "echo after the upgrade");
        assert!(relay_let_go(&relay, 5).await, "{:?}", relay.server.stats());
    })
    .await;
}

/// No punch at all: the relay path carries the data and closes with the stream.
#[tokio::test(flavor = "multi_thread")]
async fn closing_the_stream_closes_the_relay_channel() {
    within(async {
        let (_nodes, bootstrap) = cluster(4).await;
        let relay = announced_relay(&bootstrap).await;
        let key = KeyPair::from_seed([0x96; 32]);
        let mut sopts = ServerOpts::default();
        sopts.relay_through = Some(RelayThrough::Key(relay.public_key));
        echo_one(serve_key_opts(&bootstrap, &key, true, sopts).await);
        let mut o = ConnectOpts::default();
        o.local_connection = false;
        o.relay_through = Some(RelayThrough::Key(relay.public_key));
        o.holepunch = Some(HolepunchFilter::new(|_| false));
        let (_client, mut conn) = dial(bootstrap, key.public_key, o).await;

        let path = conn.relay_path().expect("relay link");
        assert_eq!(*path.borrow(), RelayPath::Relayed);
        let (sent, got) = echo_roundtrip(&mut conn, 64 * 1024, 0).await;
        assert_eq!(sent, got);
        assert_eq!(relay.server.stats().pairings_active, 1);

        conn.stream.shutdown().await.expect("shutdown");
        drop(conn);
        assert!(relay_let_go(&relay, 5).await, "{:?}", relay.server.stats());
        // The two winning sessions close with the stream; the slots of the client's other
        // handshakes never open and are abandoned when their stream times out (about 12 s).
        assert!(
            relay_sessions_closed(&relay, 30).await,
            "{:?}",
            relay.server.stats()
        );
    })
    .await;
}
