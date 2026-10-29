use super::*;
use crate::conformance::key_pair;
use crate::mock::test_util::{node, nothing_arrives, serve};
use crate::mock::{MockConfig, MockFactory, RecordFor};
use townjump_dht::{DhtFactory, DhtServer};

const SERVER_TAG: u8 = 0x71;

#[tokio::test]
async fn create_event_and_anonymous_keys_differ() {
    let factory = MockFactory::new();
    let first = factory.create(None).await.unwrap();
    let second = factory.create(None).await.unwrap();
    assert_ne!(first.public_key(), second.public_key());
    assert_eq!(first.public_key()[..8], 0u64.to_be_bytes());
    assert_eq!(second.public_key()[8..], [0xA0; 24]);
    let again = MockFactory::new().create(None).await.unwrap();
    assert_eq!(again.public_key(), first.public_key());
    let named = factory.create(Some(key_pair(3))).await.unwrap();
    assert_eq!(named.public_key(), key_pair(3).public_key);
    let created = factory.trace().named("HyperDHT.new");
    assert_eq!(created.len(), 3);
    assert_eq!(created[0]["keyPair"], serde_json::Value::Null);
    assert_eq!(created[2]["opts"], serde_json::json!(["keyPair"]));
}

#[tokio::test]
async fn put_returns_seq_none_is_zero() {
    let dht = node(&MockFactory::new(), 1).await;
    let kp = key_pair(1);
    assert_eq!(dht.mutable_put(&kp, b"a", None).await.unwrap(), 0);
    assert_eq!(dht.mutable_put(&kp, b"b", Some(7)).await.unwrap(), 7);
}

#[tokio::test]
async fn get_returns_highest_seq() {
    let dht = node(&MockFactory::new(), 1).await;
    let kp = key_pair(1);
    for (seq, value) in [(2, "two"), (5, "five"), (3, "three")] {
        dht.mutable_put(&kp, value.as_bytes(), Some(seq))
            .await
            .unwrap();
    }
    let record = dht
        .mutable_get(kp.public_key, Some(true))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        record,
        MutableRecord {
            seq: 5,
            value: b"five".to_vec()
        }
    );
}

#[tokio::test]
async fn get_unknown_is_none() {
    let dht = node(&MockFactory::new(), 1).await;
    assert_eq!(dht.mutable_get([4; 32], None).await.unwrap(), None);
}

#[tokio::test]
async fn get_canned_first() {
    let config = MockConfig::default().record(RecordFor::Any, "canned");
    let dht = node(&MockFactory::with_config(config), 1).await;
    let kp = key_pair(1);
    dht.mutable_put(&kp, b"stored", Some(9)).await.unwrap();
    let record = dht.mutable_get(kp.public_key, None).await.unwrap().unwrap();
    assert_eq!(
        record,
        MutableRecord {
            seq: 3,
            value: b"canned".to_vec()
        }
    );
}

#[tokio::test]
async fn canned_for_other_key_ignored() {
    let config = MockConfig::default().record(RecordFor::Key([8; 32]), "canned");
    let dht = node(&MockFactory::with_config(config), 1).await;
    assert_eq!(
        dht.mutable_get(key_pair(1).public_key, None).await.unwrap(),
        None
    );
    let hit = dht.mutable_get([8; 32], None).await.unwrap().unwrap();
    assert_eq!(hit.value, b"canned");
}

#[tokio::test]
async fn records_shared_within_factory_not_between() {
    let factory = MockFactory::new();
    let (writer, reader) = (node(&factory, 1).await, node(&factory, 2).await);
    writer
        .mutable_put(&key_pair(1), b"v", Some(1))
        .await
        .unwrap();
    let seen = reader
        .mutable_get(key_pair(1).public_key, None)
        .await
        .unwrap();
    assert_eq!(seen.unwrap().value, b"v");
    let stranger = node(&MockFactory::new(), 3).await;
    assert_eq!(
        stranger
            .mutable_get(key_pair(1).public_key, None)
            .await
            .unwrap(),
        None
    );
}

fn is_destroyed(error: DhtError) -> bool {
    matches!(error, DhtError::Destroyed)
}

#[tokio::test]
async fn destroy_event_and_calls_fail() {
    let factory = MockFactory::new();
    let dht = node(&factory, 1).await;
    let (server, _incoming) = serve(&dht, SERVER_TAG).await;
    dht.destroy().await.unwrap();
    assert!(is_destroyed(
        dht.create_server(ServerOpts::default()).unwrap_err()
    ));
    assert!(is_destroyed(
        server.listen(key_pair(SERVER_TAG)).await.unwrap_err()
    ));
    assert!(is_destroyed(
        dht.connect([5; 32], ConnectOpts::default())
            .await
            .unwrap_err()
    ));
    assert!(is_destroyed(
        dht.mutable_put(&key_pair(1), b"v", None).await.unwrap_err()
    ));
    assert!(is_destroyed(
        dht.mutable_get([5; 32], None).await.unwrap_err()
    ));
    dht.destroy().await.unwrap();
    assert_eq!(factory.trace().named("dht.destroy").len(), 2);
    assert_eq!(factory.trace().named("connect").len(), 1);
}

#[tokio::test]
async fn destroy_closes_own_servers() {
    let factory = MockFactory::new();
    let (owner, client) = (node(&factory, 1).await, node(&factory, 2).await);
    let (_server, mut incoming) = serve(&owner, SERVER_TAG).await;
    owner.destroy().await.unwrap();
    assert!(incoming.accept().await.is_none());
    let refused = client
        .connect(key_pair(SERVER_TAG).public_key, ConnectOpts::default())
        .await;
    assert!(matches!(refused.unwrap_err(), DhtError::PeerNotFound));
}

#[tokio::test]
async fn suspend_resume_events_and_scope() {
    let factory = MockFactory::new();
    let (sleeper, awake, client) = (
        node(&factory, 1).await,
        node(&factory, 2).await,
        node(&factory, 3).await,
    );
    let (_first, mut sleeping) = serve(&sleeper, 0x72).await;
    let (_second, mut serving) = serve(&awake, 0x73).await;
    sleeper.suspend().await.unwrap();
    let opts = ConnectOpts::default();
    let refused = client
        .connect(key_pair(0x72).public_key, opts.clone())
        .await;
    assert!(matches!(
        refused.unwrap_err(),
        DhtError::ConnectionFailed(_)
    ));
    assert!(nothing_arrives(&mut sleeping).await);
    client
        .connect(key_pair(0x73).public_key, opts.clone())
        .await
        .unwrap();
    assert!(serving.accept().await.is_some());
    sleeper.resume().await.unwrap();
    client
        .connect(key_pair(0x72).public_key, opts)
        .await
        .unwrap();
    assert!(sleeping.accept().await.is_some());
    assert_eq!(factory.trace().named("dht.suspend").len(), 1);
    assert_eq!(factory.trace().named("dht.resume").len(), 1);
}

mod to_array {
    use super::*;
    use std::net::SocketAddrV4;
    use townjump_dht::Dht;

    fn seeds() -> Vec<SocketAddrV4> {
        ["1.1.1.1:1", "2.2.2.2:2", "3.3.3.3:3"]
            .iter()
            .map(|s| s.parse().unwrap())
            .collect()
    }

    #[tokio::test]
    async fn to_array_echoes_seeded_nodes() {
        let mut factory = MockFactory::new();
        factory.set_nodes(seeds());
        let dht = factory.create(None).await.unwrap();
        assert_eq!(dht.to_array(None).await.unwrap(), seeds());
        assert_eq!(factory.seeded_nodes(), Some(seeds()));
    }

    #[tokio::test]
    async fn to_array_respects_limit() {
        let mut factory = MockFactory::new();
        factory.set_nodes(seeds());
        let dht = factory.create(None).await.unwrap();
        assert_eq!(dht.to_array(Some(2)).await.unwrap(), seeds()[..2]);
    }

    #[tokio::test]
    async fn to_array_unseeded_is_empty() {
        let factory = MockFactory::new();
        let dht = factory.create(None).await.unwrap();
        assert!(dht.to_array(None).await.unwrap().is_empty());
        assert_eq!(factory.seeded_nodes(), None);
    }

    #[tokio::test]
    async fn to_array_after_destroy_fails() {
        let dht = MockFactory::new().create(None).await.unwrap();
        dht.destroy().await.unwrap();
        let got = dht.to_array(None).await;
        assert!(
            matches!(got, Err(townjump_dht::DhtError::Destroyed)),
            "{got:?}"
        );
    }
}
