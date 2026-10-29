use super::*;
use crate::conformance::key_pair;
use crate::mock::test_util::{node, nothing_arrives, serve, serve_with};
use crate::mock::{MockConfig, MockFactory};
use serde_json::{Value, json};
use townjump_dht::{ConnectOpts, Dht, ServerOpts};

const TAG: u8 = 0x81;

type Seen = Arc<Mutex<Vec<[u8; 32]>>>;

/// Firewall that remembers its arguments and rejects everyone except `allowed`.
fn recording_firewall(allowed: [u8; 32]) -> (Firewall, Seen) {
    let seen: Seen = Arc::default();
    let log = seen.clone();
    let deny = move |pk: &[u8; 32]| {
        lock(&log).push(*pk);
        *pk != allowed
    };
    (Arc::new(deny), seen)
}

fn names(events: &[Value]) -> Vec<&str> {
    events.iter().map(|e| e["ev"].as_str().unwrap()).collect()
}

#[tokio::test]
async fn listen_events_and_probe_args() {
    let factory = MockFactory::new();
    let dht = node(&factory, 1).await;
    let (firewall, seen) = recording_firewall(key_pair(TAG).public_key);
    let opts = ServerOpts {
        firewall: Some(firewall),
        reusable_socket: true,
        ..Default::default()
    };
    let _served = serve_with(&dht, TAG, opts).await;
    let events = factory.trace().events();
    let order = [
        "HyperDHT.new",
        "createServer",
        "server.listen",
        "firewall.probe",
    ];
    assert_eq!(names(&events), order);
    assert_eq!(*lock(&seen), vec![key_pair(TAG).public_key, [7; 32]]);
    assert_eq!(
        events[3],
        json!({ "ev": "firewall.probe", "ownKey": false, "otherKey": true })
    );
}

#[tokio::test]
async fn public_server_no_probe() {
    let factory = MockFactory::new();
    let dht = node(&factory, 1).await;
    let _served = serve(&dht, TAG).await;
    assert!(factory.trace().named("firewall.probe").is_empty());
    assert_eq!(
        factory.trace().named("createServer")[0]["firewall"],
        "boolean"
    );
}

fn backend_text(error: DhtError) -> String {
    match error {
        DhtError::Backend(text) => text,
        other => panic!("expected a Backend error, got {other:?}"),
    }
}

#[tokio::test]
async fn listen_twice_and_after_close_fail() {
    let dht = node(&MockFactory::new(), 1).await;
    let (server, _incoming) = serve(&dht, TAG).await;
    let twice = server.listen(key_pair(TAG)).await.unwrap_err();
    assert_eq!(backend_text(twice), "Server already listening");
    server.close().await.unwrap();
    let closed = server.listen(key_pair(TAG)).await.unwrap_err();
    assert_eq!(backend_text(closed), "Server closed");
    let (fresh, _rx) = dht.create_server(ServerOpts::default()).unwrap();
    fresh.close().await.unwrap();
    let never = fresh.listen(key_pair(TAG)).await.unwrap_err();
    assert_eq!(backend_text(never), "Server closed");
}

#[tokio::test]
async fn close_ends_incoming_frees_key_no_event() {
    let factory = MockFactory::new();
    let (owner, client) = (node(&factory, 1).await, node(&factory, 2).await);
    let (server, mut incoming) = serve(&owner, TAG).await;
    let before = factory.trace().events().len();
    server.close().await.unwrap();
    server.close().await.unwrap();
    assert!(incoming.accept().await.is_none());
    let target = key_pair(TAG).public_key;
    let refused = client.connect(target, ConnectOpts::default()).await;
    assert!(matches!(refused.unwrap_err(), DhtError::PeerNotFound));
    assert_eq!(
        factory.trace().events().len(),
        before + 1,
        "close writes no events, only connect was added"
    );
}

#[tokio::test]
async fn second_server_replaces_first() {
    let factory = MockFactory::new();
    let (first_node, second_node) = (node(&factory, 1).await, node(&factory, 2).await);
    let client = node(&factory, 3).await;
    let (first, mut first_in) = serve(&first_node, TAG).await;
    let (_second, mut second_in) = serve(&second_node, TAG).await;
    let target = key_pair(TAG).public_key;
    client
        .connect(target, ConnectOpts::default())
        .await
        .unwrap();
    assert!(second_in.accept().await.is_some());
    assert!(nothing_arrives(&mut first_in).await);
    first.close().await.unwrap();
    client
        .connect(target, ConnectOpts::default())
        .await
        .unwrap();
    assert!(
        second_in.accept().await.is_some(),
        "closing the first does not remove the second"
    );
}

#[tokio::test]
async fn fake_conn_after_probe() {
    let config = MockConfig::default().fake_conn(true);
    let factory = MockFactory::with_config(config);
    let dht = node(&factory, 1).await;
    let (firewall, _seen) = recording_firewall(key_pair(TAG).public_key);
    let opts = ServerOpts {
        firewall: Some(firewall),
        reusable_socket: true,
        ..Default::default()
    };
    let (_server, mut incoming) = serve_with(&dht, TAG, opts).await;
    let conn = incoming.accept().await.unwrap();
    assert_eq!(conn.remote_public_key, [9; 32]);
    let order = [
        "HyperDHT.new",
        "createServer",
        "server.listen",
        "firewall.probe",
    ];
    assert_eq!(names(&factory.trace().events()), order);
}
