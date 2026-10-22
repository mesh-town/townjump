// Tests for the `vectors` module: `load_value` opens every file under `testvectors/` and strips `_meta`.
use super::*;
use std::path::Path;

const DIRS: [&str; 5] = ["holesail", "hyperdht", "dht-rpc", "deltas", "secret-stream"];
const CLI_FILES: [&str; 3] = [
    "holesail-cli/cli-matrix.json",
    "holesail-cli/keyderivation-vectors.json",
    "holesail-cli/minimist-vectors.json",
];

fn json_files(dir: &str) -> Vec<String> {
    let full = Path::new(ROOT).join(dir);
    let entries = std::fs::read_dir(&full).unwrap_or_else(|e| panic!("{}: {e}", full.display()));
    let mut names: Vec<String> = entries
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".json"))
        .map(|name| format!("{dir}/{name}"))
        .collect();
    names.sort();
    names
}

#[test]
fn load_value_opens_every_json_in_vector_dirs() {
    for dir in DIRS {
        let files = json_files(dir);
        assert!(!files.is_empty(), "no *.json in testvectors/{dir}");
        for rel in files {
            let _ = load_value(&rel);
        }
    }
}

#[test]
fn load_value_opens_holesail_cli_files() {
    for rel in CLI_FILES {
        let _ = load_value(rel);
    }
}

#[test]
fn load_value_strips_top_level_meta() {
    let value = load_value("holesail/url-parser.json");
    let object = value.as_object().expect("url-parser.json is an object");
    assert!(!object.contains_key("_meta"));
    assert!(object.contains_key("vectors"));
}

#[test]
fn load_keeps_top_level_array() {
    let cases: Vec<serde_json::Value> = load("holesail-cli/cli-matrix.json");
    assert!(!cases.is_empty());
}

#[test]
#[should_panic(expected = "node testvectors/gen/all.mjs")]
fn load_value_panics_with_regeneration_hint_when_file_is_missing() {
    let _ = load_value("holesail/no-such-file.json");
}
