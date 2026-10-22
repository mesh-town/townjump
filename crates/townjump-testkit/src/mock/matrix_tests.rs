// Comparison of the mock trace with the JS trace `testvectors/holesail-cli/cli-matrix.json`: the same calls in the same order.
use crate::mock::{MockConfig, MockFactory, strip_untracked};
use crate::vectors;
use serde_json::Value;
use std::sync::Arc;
use townjump_dht::{Dht, DhtFactory, DhtServer, KeyPair, ServerOpts};

const PUB_REC: &str = r#"{"host":"10.1.2.3","port":4000,"udp":false}"#;

fn matrix_case(id: &str) -> Value {
    let matrix = vectors::load_value("holesail-cli/cli-matrix.json");
    let found = matrix.as_array().unwrap().iter().find(|c| c["id"] == id);
    found
        .unwrap_or_else(|| panic!("no case {id} in the matrix"))
        .clone()
}

/// JS trace of a case without the events the mock does not write.
fn expected_trace(id: &str) -> Vec<Value> {
    let trace = matrix_case(id)["trace"].as_array().unwrap().clone();
    strip_untracked(&trace)
}

fn first_event<'a>(trace: &'a [Value], ev: &str) -> &'a Value {
    trace
        .iter()
        .find(|e| e["ev"] == ev)
        .unwrap_or_else(|| panic!("no {ev} in the trace"))
}

fn key_of(text: &str) -> [u8; 32] {
    hex::decode(text).unwrap().try_into().unwrap()
}

/// Pair with the public key from the trace; the mock does not check the secret.
fn pair_from(trace: &[Value], ev: &str) -> KeyPair {
    let public_key = key_of(first_event(trace, ev)["publicKey"].as_str().unwrap());
    KeyPair {
        public_key,
        secret_key: [0; 64],
    }
}

fn put_value(trace: &[Value]) -> Vec<u8> {
    first_event(trace, "mutablePut")["value"]
        .as_str()
        .unwrap()
        .as_bytes()
        .to_vec()
}

/// `--live` calls: create a node, a server, listen, `mutableGet`, `mutablePut`.
async fn replay_live(
    factory: &MockFactory,
    trace: &[Value],
    private: bool,
) -> townjump_dht::Incoming {
    let kp = pair_from(trace, "server.listen");
    let dht = factory.create(None).await.unwrap();
    let own = kp.public_key;
    let firewall = private.then(|| Arc::new(move |pk: &[u8; 32]| *pk != own) as _);
    let (server, incoming) = dht
        .create_server(ServerOpts {
            firewall,
            reusable_socket: true,
            relay_through: None,
        })
        .unwrap();
    server.listen(kp.clone()).await.unwrap();
    dht.mutable_get(own, Some(true)).await.unwrap();
    dht.mutable_put(&kp, &put_value(trace), None).await.unwrap();
    incoming
}

fn assert_same_trace(factory: &MockFactory, id: &str) {
    assert_eq!(factory.trace().events(), expected_trace(id), "case {id}");
}

#[tokio::test]
async fn live_private() {
    let factory = MockFactory::new();
    replay_live(&factory, &expected_trace("live-private"), true).await;
    assert_same_trace(&factory, "live-private");
}

#[tokio::test]
async fn live_key40_public() {
    let factory = MockFactory::new();
    replay_live(&factory, &expected_trace("live-key40-public"), false).await;
    assert_same_trace(&factory, "live-key40-public");
    assert!(factory.trace().named("firewall.probe").is_empty());
}

#[tokio::test]
async fn conn_url_s000() {
    let trace = expected_trace("conn-url-s000");
    let kp = pair_from(&trace, "mutableGet");
    let factory = MockFactory::new();
    let dht = factory.create(Some(kp.clone())).await.unwrap();
    dht.mutable_get(kp.public_key, None).await.unwrap();
    assert_same_trace(&factory, "conn-url-s000");
}

#[tokio::test]
async fn lookup_none() {
    let trace = expected_trace("lookup-none");
    let gets: Vec<[u8; 32]> = trace
        .iter()
        .filter(|e| e["ev"] == "mutableGet")
        .map(|e| key_of(e["publicKey"].as_str().unwrap()))
        .collect();
    let factory = MockFactory::new();
    let dht = factory.create(None).await.unwrap();
    for pk in &gets {
        assert_eq!(dht.mutable_get(*pk, Some(true)).await.unwrap(), None);
    }
    dht.destroy().await.unwrap();
    assert_eq!(gets.len(), 2);
    assert_same_trace(&factory, "lookup-none");
}

#[tokio::test]
async fn lookup_found_public() {
    let trace = expected_trace("lookup-found-public");
    let pk = key_of(
        first_event(&trace, "mutableGet")["publicKey"]
            .as_str()
            .unwrap(),
    );
    let env = [("MOCK_RECORD_FOR", "*"), ("MOCK_RECORD", PUB_REC)];
    let factory = MockFactory::with_config(MockConfig::from_env_pairs(env));
    let dht = factory.create(None).await.unwrap();
    let record = dht.mutable_get(pk, Some(true)).await.unwrap().unwrap();
    dht.destroy().await.unwrap();
    assert_eq!(
        (record.seq, record.value.as_slice()),
        (3, PUB_REC.as_bytes())
    );
    assert_same_trace(&factory, "lookup-found-public");
}

#[tokio::test]
async fn live_fake_conn() {
    let factory = MockFactory::with_config(MockConfig::from_env_pairs([("MOCK_FAKE_CONN", "1")]));
    let mut incoming = replay_live(&factory, &expected_trace("live-fake-conn"), true).await;
    assert_same_trace(&factory, "live-fake-conn");
    assert_eq!(incoming.accept().await.unwrap().remote_public_key, [9; 32]);
}

#[test]
fn matrix_has_no_connect_or_suspend() {
    let matrix = vectors::load_value("holesail-cli/cli-matrix.json");
    let cases = matrix.as_array().unwrap();
    assert_eq!(cases.len(), 95);
    for case in cases {
        for event in case["trace"].as_array().unwrap() {
            let name = event["ev"].as_str().unwrap();
            assert!(
                !matches!(name, "connect" | "dht.suspend" | "dht.resume"),
                "{}: {name}",
                case["id"]
            );
        }
    }
}
