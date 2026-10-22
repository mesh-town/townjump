//! Cluster tests of `Server` as a whole: the firewall scenarios of the private
//! and public holesail modes, data through a connection, and the life cycle
//! (`listen`, `close`). Real Rust clients connect through a local DHT cluster.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use crate::hyperdht::{ConnectOpts, HyperDhtError, KeyPair, PeerConnection};
use crate::hyperdht_messages::FIREWALL_UNKNOWN;
use crate::server::test_net::{
    Cluster, Node, Served, cluster, echo, hello, key, node, raw_handshake, roundtrip, serve,
    sha256, start, within,
};
use crate::server::{Firewall, ServerOpts};

/// Options whose firewall counts its calls and refuses when `refuse(client key)` is true.
fn counting(
    refuse: impl Fn(&[u8; 32]) -> bool + Send + Sync + 'static,
) -> (ServerOpts, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&calls);
    let firewall: Firewall = Arc::new(move |input| {
        seen.fetch_add(1, Ordering::SeqCst);
        refuse(input.remote_public_key)
    });
    (ServerOpts::default().with_firewall(firewall), calls)
}

/// Private mode of holesail: only the server's own key pair gets in.
fn private(server_pk: [u8; 32]) -> (ServerOpts, Arc<AtomicUsize>) {
    counting(move |client| *client != server_pk)
}

async fn client_node(cl: &Cluster) -> Node {
    node(cl, false).await
}

async fn connect(
    client: &Node,
    kp: &KeyPair,
    server_pk: [u8; 32],
) -> Result<PeerConnection, HyperDhtError> {
    client.handle.connect(kp, server_pk, &client.rt).await
}

/// The next connection the server accepted (10 s at most).
async fn accepted(server: &mut Served) -> PeerConnection {
    let next = tokio::time::timeout(Duration::from_secs(10), server.incoming.recv()).await;
    next.expect("no incoming connection in 10 s")
        .expect("incoming channel closed")
}

/// A raw handshake of `client_key` through a cluster node: `true` when the server answered.
async fn answered(cl: &Cluster, client_key: &KeyPair, server_pk: [u8; 32]) -> bool {
    let prober = client_node(cl).await;
    let (_nw, noise) = hello(client_key, server_pk, FIREWALL_UNKNOWN, Some(7));
    raw_handshake(&prober, server_pk, &noise, &cl.addrs[0])
        .await
        .is_some()
}

/// The server refused: no answer, nothing incoming for 2 s, the callback ran once.
async fn assert_refused(
    cl: &Cluster,
    server: &mut Served,
    calls: &AtomicUsize,
    client_key: &KeyPair,
) {
    assert!(
        !answered(cl, client_key, key(1).public_key).await,
        "a refused client gets no answer"
    );
    let nothing = tokio::time::timeout(Duration::from_secs(2), server.incoming.recv()).await;
    assert!(
        nothing.is_err(),
        "a refused client must not produce a connection"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1, "the callback ran once");
}

#[tokio::test]
async fn firewall_1_private_same_key_connects() {
    within(async {
        let cl = cluster(3).await;
        let own = key(1);
        let (opts, calls) = private(own.public_key);
        let mut server = serve(node(&cl, false).await, &own, opts).await;
        let conn = connect(&client_node(&cl).await, &own, own.public_key).await;
        // The client end is kept until the server has accepted (its header is read lazily).
        let conn = conn.expect("same key connects");
        assert_eq!(conn.remote_public_key, own.public_key);
        assert_eq!(
            accepted(&mut server).await.remote_public_key,
            own.public_key
        );
        assert!(
            calls.load(Ordering::SeqCst) >= 1,
            "the callback saw the client"
        );
    })
    .await;
}

#[tokio::test]
async fn firewall_2_private_other_key_refused() {
    within(async {
        let cl = cluster(3).await;
        let (opts, calls) = private(key(1).public_key);
        let mut server = serve(node(&cl, false).await, &key(1), opts).await;
        assert_refused(&cl, &mut server, &calls, &key(2)).await;
    })
    .await;
}

#[tokio::test]
async fn firewall_3_private_random_client_refused() {
    within(async {
        let cl = cluster(3).await;
        let (opts, calls) = private(key(1).public_key);
        let mut server = serve(node(&cl, false).await, &key(1), opts).await;
        assert_refused(&cl, &mut server, &calls, &KeyPair::generate()).await;
    })
    .await;
}

#[tokio::test]
async fn firewall_4_public_accepts_any() {
    within(async {
        let cl = cluster(3).await;
        let mut server = serve(node(&cl, false).await, &key(1), ServerOpts::default()).await;
        let stranger = KeyPair::generate();
        let conn = connect(&client_node(&cl).await, &stranger, key(1).public_key).await;
        assert!(conn.is_ok(), "a public server accepts any client");
        assert_eq!(
            accepted(&mut server).await.remote_public_key,
            stranger.public_key
        );
    })
    .await;
}

#[tokio::test]
async fn firewall_5_panic_rejects_and_server_survives() {
    within(async {
        let cl = cluster(3).await;
        let (a, b) = (key(2), key(3));
        let bad = a.public_key;
        let (opts, calls) = counting(move |client| {
            assert_ne!(*client, bad, "callback panics for client A");
            false
        });
        let mut server = serve(node(&cl, false).await, &key(1), opts).await;
        assert!(
            !answered(&cl, &a, key(1).public_key).await,
            "a panic refuses the client"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let conn = connect(&client_node(&cl).await, &b, key(1).public_key).await;
        assert!(conn.is_ok(), "the server survives a panicking callback");
        assert_eq!(accepted(&mut server).await.remote_public_key, b.public_key);
    })
    .await;
}

#[tokio::test]
async fn echo_1mb_roundtrip() {
    within(async {
        let cl = cluster(3).await;
        let mut server = serve(node(&cl, false).await, &key(1), ServerOpts::default()).await;
        let conn = connect(&client_node(&cl).await, &key(2), key(1).public_key).await;
        let incoming = accepted(&mut server).await;
        tokio::spawn(echo(incoming));
        let bytes: Vec<u8> = (0..1_048_576usize).map(|i| (i % 251) as u8).collect();
        let want = sha256(&bytes);
        assert_eq!(roundtrip(conn.expect("connects"), bytes).await, want);
    })
    .await;
}

#[tokio::test]
async fn two_connects_one_key_give_two_incoming() {
    within(async {
        let cl = cluster(3).await;
        let mut server = serve(node(&cl, false).await, &key(1), ServerOpts::default()).await;
        let client = client_node(&cl).await;
        let first = connect(&client, &key(2), key(1).public_key).await;
        let second = connect(&client, &key(2), key(1).public_key).await;
        assert!(first.is_ok() && second.is_ok(), "both connects succeed");
        for _ in 0..2 {
            assert_eq!(
                accepted(&mut server).await.remote_public_key,
                key(2).public_key
            );
        }
    })
    .await;
}

#[tokio::test]
async fn listen_twice_fails() {
    within(async {
        let cl = cluster(3).await;
        let server = serve(node(&cl, false).await, &key(1), ServerOpts::default()).await;
        let again = server.server.listen(key(1)).await;
        assert!(
            matches!(again, Err(HyperDhtError::AlreadyListening)),
            "got {again:?}"
        );
    })
    .await;
}

#[tokio::test]
async fn close_unannounces_and_refuses() {
    within(async {
        let cl = cluster(3).await;
        let server = serve(node(&cl, false).await, &key(1), ServerOpts::default()).await;
        server.server.close().await.expect("close");
        server.server.close().await.expect("close is idempotent");
        let again = server.server.listen(key(1)).await;
        assert!(
            matches!(again, Err(HyperDhtError::ServerClosed)),
            "got {again:?}"
        );
        assert!(
            !answered(&cl, &key(2), key(1).public_key).await,
            "a closed server is silent"
        );
        let late = connect(&client_node(&cl).await, &key(2), key(1).public_key).await;
        assert!(late.is_err(), "a closed server accepts nobody");
    })
    .await;
}

#[tokio::test]
async fn close_before_listen_then_listen_fails() {
    within(async {
        let cl = cluster(3).await;
        let idle = start(node(&cl, false).await, ServerOpts::default());
        idle.server.close().await.expect("close of an idle server");
        let late = idle.server.listen(key(1)).await;
        assert!(
            matches!(late, Err(HyperDhtError::ServerClosed)),
            "got {late:?}"
        );
    })
    .await;
}

#[tokio::test]
async fn holepunch_firewalled_server_accepts_rust_client() {
    within(async {
        let cl = cluster(3).await;
        let mut server = serve(node(&cl, true).await, &key(1), ServerOpts::default()).await;
        let client = node(&cl, true).await;
        let forced = ConnectOpts {
            local_connection: false,
            ..Default::default()
        };
        let conn = client
            .handle
            .connect_with_options(&key(2), key(1).public_key, &[], &client.rt, forced)
            .await;
        let conn = conn.expect("connects by holepunch");
        assert_eq!(conn.remote_public_key, key(1).public_key);
        assert_eq!(
            accepted(&mut server).await.remote_public_key,
            key(2).public_key
        );
    })
    .await;
}
