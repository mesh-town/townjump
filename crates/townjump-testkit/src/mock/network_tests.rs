use super::*;
use crate::conformance::key_pair;
use crate::mock::test_util::{node, nothing_arrives, serve, serve_with};
use std::sync::Mutex;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::task::JoinSet;
use townjump_dht::{ConnectOpts, Dht, ServerOpts};

const TAG: u8 = 0x91;
fn connect_opts() -> ConnectOpts {
    ConnectOpts {
        reusable_socket: false,
        ..Default::default()
    }
}

fn target() -> [u8; 32] {
    key_pair(TAG).public_key
}

#[tokio::test]
async fn connect_pairs_streams_with_identities() {
    let factory = MockFactory::new();
    let (owner, client) = (node(&factory, 1).await, node(&factory, 2).await);
    let (_server, mut incoming) = serve(&owner, TAG).await;
    let mut near = client.connect(target(), connect_opts()).await.unwrap();
    let mut far = incoming.accept().await.unwrap();
    assert_eq!(near.remote_public_key, target());
    assert_eq!(far.remote_public_key, key_pair(2).public_key);
    near.stream.write_all(b"ping").await.unwrap();
    far.stream.write_all(b"pong").await.unwrap();
    let (mut ping, mut pong) = ([0u8; 4], [0u8; 4]);
    far.stream.read_exact(&mut ping).await.unwrap();
    near.stream.read_exact(&mut pong).await.unwrap();
    assert_eq!((&ping, &pong), (b"ping", b"pong"));
}

#[tokio::test]
async fn unregistered_strict_peer_not_found() {
    let client = node(&MockFactory::new(), 2).await;
    let refused = client.connect([6; 32], connect_opts()).await;
    assert!(matches!(refused.unwrap_err(), DhtError::PeerNotFound));
}

#[tokio::test]
async fn lenient_returns_stub() {
    let factory = MockFactory::with_config(MockConfig::default().lenient_connect(true));
    let client = node(&factory, 2).await;
    let mut conn = client.connect([6; 32], connect_opts()).await.unwrap();
    assert_eq!(conn.remote_public_key, [6; 32]);
    conn.stream.write_all(b"into the void").await.unwrap();
    let mut byte = [0u8; 1];
    let read = tokio::time::timeout(Duration::from_millis(50), conn.stream.read(&mut byte)).await;
    assert!(read.is_err(), "read from the stub returned");
}

#[tokio::test]
async fn firewall_refuses() {
    let factory = MockFactory::new();
    let (owner, client) = (node(&factory, 1).await, node(&factory, 2).await);
    let opts = ServerOpts {
        firewall: Some(Arc::new(|_| true)),
        reusable_socket: true,
        ..Default::default()
    };
    let (_server, mut incoming) = serve_with(&owner, TAG, opts).await;
    let refused = client.connect(target(), connect_opts()).await.unwrap_err();
    let text = refused.to_string();
    assert_eq!(text, "PEER_CONNECTION_FAILED: Could not connect to peer");
    assert!(nothing_arrives(&mut incoming).await);
}

#[tokio::test]
async fn firewall_receives_client_identity() {
    let factory = MockFactory::new();
    let (owner, client) = (node(&factory, 1).await, node(&factory, 2).await);
    let seen: Arc<Mutex<Vec<[u8; 32]>>> = Arc::default();
    let log = seen.clone();
    let watch = move |pk: &[u8; 32]| {
        log.lock().unwrap().push(*pk);
        false
    };
    let opts = ServerOpts {
        firewall: Some(Arc::new(watch)),
        reusable_socket: true,
        ..Default::default()
    };
    let (_server, mut incoming) = serve_with(&owner, TAG, opts).await;
    client.connect(target(), connect_opts()).await.unwrap();
    assert!(incoming.accept().await.is_some());
    let last = *seen.lock().unwrap().last().unwrap();
    assert_eq!(last, key_pair(2).public_key);
}

#[tokio::test]
async fn many_connects_one_identity() {
    let factory = MockFactory::new();
    let (owner, client) = (node(&factory, 1).await, node(&factory, 2).await);
    let (_server, mut incoming) = serve(&owner, TAG).await;
    let client = Arc::new(client);
    let mut pending = JoinSet::new();
    for _ in 0..8 {
        let client = client.clone();
        pending.spawn(async move { client.connect(target(), connect_opts()).await });
    }
    while let Some(done) = pending.join_next().await {
        done.unwrap().unwrap();
    }
    for _ in 0..8 {
        let conn = incoming.accept().await.unwrap();
        assert_eq!(conn.remote_public_key, key_pair(2).public_key);
    }
    assert!(nothing_arrives(&mut incoming).await);
}

#[tokio::test]
async fn connect_event_even_on_error() {
    let factory = MockFactory::new();
    let client = node(&factory, 2).await;
    let _ = client
        .connect(
            [6; 32],
            ConnectOpts {
                reusable_socket: true,
                ..Default::default()
            },
        )
        .await;
    let events = factory.trace().named("connect");
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0]["opts"],
        serde_json::json!({ "reusableSocket": true })
    );
}

#[tokio::test]
async fn dropped_incoming_refuses() {
    let factory = MockFactory::new();
    let (owner, client) = (node(&factory, 1).await, node(&factory, 2).await);
    let (_server, incoming) = serve(&owner, TAG).await;
    drop(incoming);
    let refused = client.connect(target(), connect_opts()).await.unwrap_err();
    assert!(matches!(refused, DhtError::ConnectionFailed(_)));
}

#[tokio::test]
async fn duplex_holds_one_mib() {
    let factory = MockFactory::new();
    let (owner, client) = (node(&factory, 1).await, node(&factory, 2).await);
    let (_server, mut incoming) = serve(&owner, TAG).await;
    let mut conn = client.connect(target(), connect_opts()).await.unwrap();
    let _held = incoming.accept().await.unwrap();
    let block = vec![0x5a; 1 << 20];
    let write = tokio::time::timeout(Duration::from_secs(1), conn.stream.write_all(&block)).await;
    write
        .expect("writing 1 MiB without a reader blocked")
        .unwrap();
}
