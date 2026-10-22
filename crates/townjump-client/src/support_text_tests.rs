// Text and trace helpers for client tests: z32, log line parsing, mock trace normalisation, events and matrix cases.
// Included from support_tests.rs and re-exported from there.
use super::key32;
use serde_json::{Value, json};
use townjump_testkit::mock::strip_untracked;
use townjump_testkit::vectors;

/// `z32.encode`: five bits per character, the tail is padded with zeros.
pub fn z32_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"ybndrfg8ejkmcpqxot1uwisza345h769";
    let (mut out, mut acc, mut bits) = (String::new(), 0u32, 0u32);
    for &byte in bytes {
        acc = (acc << 8) | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            out.push(char::from(ALPHABET[((acc >> (bits - 5)) & 31) as usize]));
            bits -= 5;
        }
        acc &= (1 << bits) - 1;
    }
    if bits > 0 {
        out.push(char::from(ALPHABET[((acc << (5 - bits)) & 31) as usize]));
    }
    out
}

/// Log lines `[LEVEL] text`: format `<ISO> [Holesail] [LEVEL] msg`, ANSI colours stripped.
pub fn log_lines(raw: &str) -> Vec<String> {
    let plain = strip_ansi(raw);
    let rest_of = |line: &str| {
        let (_stamp, rest) = line
            .split_once(" [Holesail] ")
            .unwrap_or_else(|| panic!("log line has the wrong format: {line:?}"));
        rest.to_owned()
    };
    plain.lines().map(rest_of).collect()
}

fn strip_ansi(raw: &str) -> String {
    let mut out = String::new();
    let mut chars = raw.chars();
    while let Some(ch) = chars.next() {
        if ch != '\u{1b}' {
            out.push(ch);
            continue;
        }
        chars.by_ref().find(|c| c.is_ascii_alphabetic());
    }
    out
}

/// Strips `opts` from `HyperDHT.new`: the JS mock writes `["keyPair"]` there, ours writes `[]`.
pub fn normalized(events: Vec<Value>) -> Vec<Value> {
    let strip = |mut event: Value| {
        if event["ev"] == "HyperDHT.new" {
            event
                .as_object_mut()
                .expect("event is an object")
                .remove("opts");
        }
        event
    };
    events.into_iter().map(strip).collect()
}

/// `new HyperDHT({ keyPair })` (`pk` is the key pair public key, hex) or `new HyperDHT()`.
pub fn ev_new(pk: Option<&str>) -> Value {
    let key_pair = pk.map(|key| json!({ "publicKey": key }));
    json!({ "ev": "HyperDHT.new", "keyPair": key_pair })
}

pub fn ev_get(pk: &str, opts: Value) -> Value {
    json!({ "ev": "mutableGet", "publicKey": pk, "opts": opts })
}

pub fn ev(name: &str) -> Value {
    json!({ "ev": name })
}

/// JS trace of a matrix case without the events the mock does not write, and without `opts` of `HyperDHT.new`.
pub fn matrix_trace(id: &str) -> Vec<Value> {
    let trace = matrix_case(id)["trace"].as_array().expect("trace").clone();
    normalized(strip_untracked(&trace))
}

/// `z32(seed)` from the `HyperDHT.keyPair` event of a matrix case: the key the facade hands to a private client.
pub fn matrix_seed_z32(id: &str) -> String {
    let trace = matrix_case(id)["trace"].as_array().expect("trace").clone();
    let event = trace.iter().find(|e| e["ev"] == "HyperDHT.keyPair");
    let seed = event.expect("no HyperDHT.keyPair in the trace")["seed"].as_str();
    z32_encode(&key32(seed.expect("seed")))
}

fn matrix_case(id: &str) -> Value {
    let matrix = vectors::load_value("holesail-cli/cli-matrix.json");
    let found = matrix.as_array().expect("matrix is an array");
    let case = found.iter().find(|c| c["id"] == id);
    case.unwrap_or_else(|| panic!("no case {id} in the matrix"))
        .clone()
}
