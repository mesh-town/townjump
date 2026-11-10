//! Runs every record of the shared vector files through the public API.
use serde_json::Value;
use townjump_keys::{ClientIdentity, client_identity, parse_url, server_keys};

fn load(name: &str) -> Value {
    let path = format!(
        "{}/../../testvectors/holesail/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn input_text(v: &Value) -> String {
    match v["type"].as_str().unwrap() {
        "undefined" | "null" => String::new(),
        _ => match &v["value"] {
            Value::String(s) => s.clone(),
            Value::Bool(false) => String::new(),
            other => other.to_string(),
        },
    }
}

#[test]
fn url_parser_vectors() {
    let doc = load("url-parser.json");
    let list = doc["vectors"].as_array().unwrap();
    assert_eq!(list.len(), 49);
    for rec in list {
        let text = match rec["input"]["type"].as_str().unwrap() {
            "number" if rec["input"]["value"] == 0 => String::new(),
            _ => input_text(&rec["input"]),
        };
        let got = parse_url(&text);
        assert_eq!(got.key, rec["key"].as_str().unwrap(), "{rec}");
        assert_eq!(got.secure, rec["secure"].as_bool(), "{rec}");
    }
}

fn mode(v: &Value) -> bool {
    match v["type"].as_str().unwrap() {
        "boolean" => v["value"].as_bool().unwrap(),
        "string" => true,
        _ => false,
    }
}

fn hex_of(v: &Value) -> Vec<u8> {
    hex::decode(v.as_str().unwrap()).unwrap()
}

/// Final mode: a private marker in the connection string wins over the requested mode.
fn final_mode(parsed_secure: Option<bool>, requested: &Value) -> bool {
    parsed_secure.unwrap_or_else(|| mode(requested))
}

fn check_server(rec: &Value, rnd: &[u8], key_text: &str) {
    let inp = &rec["input"];
    let parsed = parse_url(key_text);
    let (key_text, secure) = (
        parsed.key.as_str(),
        final_mode(parsed.secure, &inp["secure"]),
    );
    let key = (inp["key"]["type"] != "undefined").then_some(key_text);
    let mut calls = 0;
    let got = server_keys(key, secure, &mut |b| {
        calls += 1;
        b.copy_from_slice(rnd);
    });
    let want_calls =
        usize::from(inp["randomBytesHex"].is_string() && key.is_none_or(str::is_empty));
    assert_eq!(calls, want_calls, "{rec}");
    let r = &rec["result"];
    assert_eq!(got.seed_hex.as_deref(), r["seedHex"].as_str(), "{rec}");
    assert_eq!(
        got.generated_seed_hex,
        r["generatedSeedHex"].as_str().unwrap()
    );
    assert_eq!(got.key_pair.public_key.to_vec(), hex_of(&r["publicKey"]));
    assert_eq!(got.key_pair.secret_key.to_vec(), hex_of(&r["secretKey"]));
    // Records with the textual mode are hybrids: the server key is not comparable.
    if inp["secure"]["type"] != "string" {
        assert_eq!(got.server_key, r["serverKey"].as_str().unwrap(), "{rec}");
    }
    assert_eq!(got.info_key, r["infoKey"].as_str().unwrap(), "{rec}");
    assert_eq!(got.url, r["url"].as_str().unwrap(), "{rec}");
}

fn check_client(rec: &Value, key_text: &str) {
    let parsed = parse_url(key_text);
    let secure = final_mode(parsed.secure, &rec["input"]["secure"]);
    let got = client_identity(&parsed.key, secure);
    if let Some(err) = rec.get("error") {
        assert_eq!(
            got.unwrap_err().to_string(),
            err["message"].as_str().unwrap()
        );
        return;
    }
    let r = &rec["result"];
    match got.unwrap() {
        ClientIdentity::Private { key_pair, seed_z32 } => {
            assert_eq!(seed_z32, r["seedZ32OrKey"].as_str().unwrap());
            assert_eq!(key_pair.public_key.to_vec(), hex_of(&r["publicKey"]));
            assert_eq!(
                key_pair.secret_key.to_vec(),
                hex_of(&r["keyPair"]["secretKey"])
            );
        }
        ClientIdentity::Public { public_key } => {
            assert_eq!(public_key, hex_of(&r["publicKey"]));
            assert!(r["keyPair"].is_null());
        }
    }
}

#[test]
fn key_derivation_vectors() {
    let doc = load("key-derivation.json");
    let rnd = hex::decode(doc["randomBytesHex"].as_str().unwrap()).unwrap();
    let list = doc["vectors"].as_array().unwrap();
    assert_eq!(list.len(), 84);
    for rec in list {
        let text = input_text(&rec["input"]["key"]);
        match rec["input"]["role"].as_str().unwrap() {
            "server" => check_server(rec, &rnd, &text),
            _ => check_client(rec, &text),
        }
    }
}
