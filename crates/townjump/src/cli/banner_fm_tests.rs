// Tests of `filemanager_banner`: stdout bytes of the `fm-*` cases of the CLI matrix and the structure (title, login line, no QR).
// JS: holesail/src/lib/stdout.js (the `type === 'filemanager'` branch).
use super::*;
use townjump_jsval::JsValue;
use townjump_testkit::vectors::load_value;

const PRIVATE_URL: &str =
    "hs://s000000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
const PUBLIC_URL: &str = "hs://0000yqooxx9u3aemh8mo5wcqq16yufu6jitouq1o4za751dger1igghy";
const SHORT_KEY: &str = "hs://s000abcdefghijklmnopqrstuvwxyz0123456789ABCD";

#[test]
fn fm_default_matches_js() {
    let got = filemanager_banner(
        &dht(true, 5409.0, PRIVATE_URL),
        &files("admin", "admin", "user"),
    );
    assert_same("fm-default", &got);
}

#[test]
fn fm_opts_public_matches_js() {
    let got = filemanager_banner(
        &dht(false, 18930.0, PUBLIC_URL),
        &files("bob", "pw", "admin"),
    );
    assert_same("fm-opts", &got);
}

#[test]
fn fm_key_matches_js() {
    let got = filemanager_banner(
        &dht(true, 18931.0, SHORT_KEY),
        &files("admin", "admin", "user"),
    );
    assert_same("fm-key", &got);
}

#[test]
fn fm_port_inuse_matches_js() {
    let got = filemanager_banner(
        &dht(true, 18932.0, PRIVATE_URL),
        &files("admin", "admin", "user"),
    );
    assert_same("fm-port-inuse", &got);
}

#[test]
fn heading_login_and_no_qr() {
    let got = filemanager_banner(
        &dht(false, 18930.0, PUBLIC_URL),
        &files("bob", "pw", "admin"),
    );
    assert!(got.contains("Holesail Filemanager Started"));
    assert!(got.contains("Username: bob Password: pw Role: admin"));
    assert!(got.contains("Connected to key: "));
    assert!(!got.contains("Connect with key"));
    assert!(got.trim_end().ends_with('┘'), "no QR after the frame");
    assert!(!got.contains('█') && !got.contains('▀'));
}

#[test]
fn numeric_password_prints_as_js_number() {
    let mut info = files("bob", "pw", "admin");
    info.password = JsValue::Number(7.0);
    let got = filemanager_banner(&dht(true, 18930.0, PRIVATE_URL), &info);
    assert!(got.contains("Password: 7 Role"), "{got}");
}

fn dht(secure: bool, port: f64, url: &str) -> HolesailInfo {
    HolesailInfo {
        kind: "server",
        state: Some("listening"),
        secure,
        port: JsValue::Number(port),
        host: JsValue::String("127.0.0.1".to_owned()),
        protocol: "tcp",
        seed: None,
        key: "MARKER-KEY".to_owned(),
        url: url.to_owned(),
        public_key: "MARKER-PUBLIC".to_owned(),
    }
}

fn files(username: &str, password: &str, role: &str) -> FilemanagerInfo {
    FilemanagerInfo {
        kind: "filemanager",
        host: JsValue::String("127.0.0.1".to_owned()),
        port: JsValue::Number(5409.0),
        role: role.to_owned(),
        username: JsValue::String(username.to_owned()),
        password: JsValue::String(password.to_owned()),
        max_request_size: 10_485_760,
        stream_buffer_size: 65_536,
    }
}

/// The banner equals the matrix case stdout byte for byte.
fn assert_same(id: &str, got: &str) {
    let matrix = load_value("holesail-cli/cli-matrix.json");
    let case = matrix
        .as_array()
        .and_then(|all| all.iter().find(|c| c["id"] == id));
    let want = case.unwrap_or_else(|| panic!("no case {id} in the matrix"))["stdout"]
        .as_str()
        .expect("stdout");
    assert_eq!(got, want, "case {id}");
}
