// JS: testvectors/holesail-cli/mock-dht.cjs (`trace(ev, data)` - one call per line below).
// Related: trace.rs stores these events, dht.rs and server.rs create them. Hex is lower case.
use serde_json::{Map, Value, json};
use townjump_dht::{ConnectOpts, ServerOpts};

pub fn hex32(key: &[u8; 32]) -> String {
    hex::encode(key)
}

/// `{ev: name, …fields}`; `ev` is inserted first, `fields` is a JSON object.
pub fn event(name: &str, fields: Value) -> Value {
    let mut map = Map::new();
    map.insert("ev".to_string(), Value::String(name.to_string()));
    if let Value::Object(extra) = fields {
        map.extend(extra);
    }
    Value::Object(map)
}

/// `new HyperDHT()` — `opts: []`; `new HyperDHT({ keyPair })` — `opts: ["keyPair"]`.
pub fn new_event(identity: Option<&[u8; 32]>) -> Value {
    let Some(pk) = identity else {
        return event("HyperDHT.new", json!({ "keyPair": null, "opts": [] }));
    };
    let fields = json!({ "keyPair": { "publicKey": hex32(pk) }, "opts": ["keyPair"] });
    event("HyperDHT.new", fields)
}

pub fn create_server_event(opts: &ServerOpts) -> Value {
    let firewall = if opts.firewall.is_some() {
        "function"
    } else {
        "boolean"
    };
    event(
        "createServer",
        json!({
            "firewall": firewall,
            "reusableSocket": opts.reusable_socket,
            "optKeys": ["firewall", "reusableSocket"],
        }),
    )
}

pub fn listen_event(public_key: &[u8; 32]) -> Value {
    event("server.listen", json!({ "publicKey": hex32(public_key) }))
}

/// Firewall answers for the own key and a foreign one (`[7; 32]`).
pub fn probe_event(own_key: bool, other_key: bool) -> Value {
    event(
        "firewall.probe",
        json!({ "ownKey": own_key, "otherKey": other_key }),
    )
}

pub fn mutable_get_event(public_key: &[u8; 32], latest: Option<bool>) -> Value {
    let opts = latest.map_or(json!({}), |flag| json!({ "latest": flag }));
    event(
        "mutableGet",
        json!({ "publicKey": hex32(public_key), "opts": opts }),
    )
}

/// The `seq` field is present only when `seq` is set, as in JS (`undefined` disappears from JSON).
pub fn mutable_put_event(public_key: &[u8; 32], value: &[u8], seq: Option<u64>) -> Value {
    let mut fields = json!({
        "publicKey": hex32(public_key),
        "value": String::from_utf8_lossy(value),
        "optKeys": if seq.is_some() { json!(["seq"]) } else { json!([]) },
    });
    if let Some(seq) = seq {
        fields["seq"] = json!(seq);
    }
    event("mutablePut", fields)
}

/// `opts` is present only with `reusable_socket` (without options - UDP, the field is absent from JSON).
pub fn connect_event(public_key: &[u8; 32], opts: &ConnectOpts) -> Value {
    let mut fields = json!({ "publicKey": hex32(public_key) });
    if opts.reusable_socket {
        fields["opts"] = json!({ "reusableSocket": true });
    }
    event("connect", fields)
}

fn plain_event(name: &str) -> Value {
    event(name, json!({}))
}

pub fn destroy_event() -> Value {
    plain_event("dht.destroy")
}

pub fn suspend_event() -> Value {
    plain_event("dht.suspend")
}

pub fn resume_event() -> Value {
    plain_event("dht.resume")
}
