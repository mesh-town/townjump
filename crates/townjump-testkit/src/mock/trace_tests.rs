use super::*;
use crate::mock::events::{self, hex32};
use serde_json::json;
use std::sync::Arc;
use townjump_dht::{ConnectOpts, ServerOpts};

fn server_opts(firewall: bool, reusable_socket: bool) -> ServerOpts {
    let firewall = firewall.then(|| Arc::new(|_: &[u8; 32]| false) as _);
    ServerOpts {
        firewall,
        reusable_socket,
        relay_through: None,
    }
}

#[test]
fn new_event_without_identity() {
    let expected = json!({ "ev": "HyperDHT.new", "keyPair": null, "opts": [] });
    assert_eq!(events::new_event(None), expected);
}

#[test]
fn new_event_with_identity() {
    let expected = json!({
        "ev": "HyperDHT.new",
        "keyPair": { "publicKey": "ab".repeat(32) },
        "opts": ["keyPair"],
    });
    assert_eq!(events::new_event(Some(&[0xab; 32])), expected);
}

#[test]
fn create_server_event() {
    let cases = [
        (true, true, "function"),
        (false, false, "boolean"),
        (true, false, "function"),
    ];
    for (firewall, reusable, name) in cases {
        let expected = json!({
            "ev": "createServer",
            "firewall": name,
            "reusableSocket": reusable,
            "optKeys": ["firewall", "reusableSocket"],
        });
        assert_eq!(
            events::create_server_event(&server_opts(firewall, reusable)),
            expected
        );
    }
}

#[test]
fn mutable_put_event() {
    let pk = [1; 32];
    let base = |seq: Option<u64>| events::mutable_put_event(&pk, b"v", seq);
    let none = json!({ "ev": "mutablePut", "publicKey": hex32(&pk), "value": "v", "optKeys": [] });
    assert_eq!(base(None), none);
    let zero = json!({
        "ev": "mutablePut", "publicKey": hex32(&pk), "value": "v", "optKeys": ["seq"], "seq": 0,
    });
    assert_eq!(base(Some(0)), zero);
    let lossy = events::mutable_put_event(&pk, &[b'a', 0xff], None);
    assert_eq!(lossy["value"], "a\u{fffd}");
}

#[test]
fn mutable_get_event() {
    let pk = [2; 32];
    let expected =
        |opts: Value| json!({ "ev": "mutableGet", "publicKey": hex32(&pk), "opts": opts });
    assert_eq!(events::mutable_get_event(&pk, None), expected(json!({})));
    assert_eq!(
        events::mutable_get_event(&pk, Some(true)),
        expected(json!({ "latest": true }))
    );
    assert_eq!(
        events::mutable_get_event(&pk, Some(false)),
        expected(json!({ "latest": false }))
    );
}

#[test]
fn connect_event() {
    let pk = [3; 32];
    let plain = events::connect_event(&pk, &ConnectOpts::default());
    assert_eq!(plain, json!({ "ev": "connect", "publicKey": hex32(&pk) }));
    let reusable = events::connect_event(
        &pk,
        &ConnectOpts {
            reusable_socket: true,
            relay_through: None,
        },
    );
    let expected = json!({
        "ev": "connect", "publicKey": hex32(&pk), "opts": { "reusableSocket": true },
    });
    assert_eq!(reusable, expected);
}

#[test]
fn plain_events() {
    let pk = [4; 32];
    assert_eq!(events::destroy_event(), json!({ "ev": "dht.destroy" }));
    assert_eq!(events::suspend_event(), json!({ "ev": "dht.suspend" }));
    assert_eq!(events::resume_event(), json!({ "ev": "dht.resume" }));
    assert_eq!(
        events::listen_event(&pk),
        json!({ "ev": "server.listen", "publicKey": hex32(&pk) })
    );
    let probe = json!({ "ev": "firewall.probe", "ownKey": false, "otherKey": true });
    assert_eq!(events::probe_event(false, true), probe);
}

#[test]
fn lines_start_with_ev() {
    let trace = MockTrace::default();
    trace.push(events::new_event(Some(&[5; 32])));
    trace.push(events::mutable_put_event(&[5; 32], b"x", Some(1)));
    trace.push(events::destroy_event());
    let lines = trace.lines();
    assert_eq!(lines.len(), 3);
    for line in &lines {
        assert!(line.starts_with("{\"ev\":\""), "{line}");
    }
    let parsed: Vec<Value> = lines
        .iter()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(parsed, trace.events());
}

#[test]
fn clones_share_and_named_filters() {
    let trace = MockTrace::default();
    let clone = trace.clone();
    clone.push(events::destroy_event());
    trace.push(events::suspend_event());
    clone.push(events::destroy_event());
    assert_eq!(trace.events().len(), 3);
    assert_eq!(trace.named("dht.destroy").len(), 2);
    assert_eq!(clone.named("dht.suspend"), vec![events::suspend_event()]);
    assert!(trace.named("nothing").is_empty());
}

#[test]
fn strip_untracked_drops_two_kinds() {
    let events = vec![
        json!({ "ev": "randomBytes", "n": 32 }),
        events::new_event(None),
        json!({ "ev": "HyperDHT.keyPair", "seed": "00", "publicKey": "11" }),
        events::destroy_event(),
    ];
    assert_eq!(
        strip_untracked(&events),
        vec![events::new_event(None), events::destroy_event()]
    );
}
