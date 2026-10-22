// Tests of `facade`: `new` and `info` by mode, keys and the connection string.
// Source: key-derivation.json, connection-string.md; pause/resume/close/lookup are in facade_calls_tests.rs.
use super::*;
use serde_json::Value;
use std::net::TcpListener;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::time::timeout;
use townjump_testkit::mock::{MockConfig, MockFactory, RecordFor, strip_untracked};
use townjump_testkit::{Capture, fixed_random, vectors};

#[path = "facade_calls_tests.rs"]
mod calls;

const WAIT: Duration = Duration::from_secs(5);
const Z52: &str = "ybndrfg8ejkmcpqxot1uwisza345h769ybndrfg8ejkmcpqxot1";
const Z52_SIXTH_S: &str = "ybndrsg8ejkmcpqxot1uwisza345h769ybndrfg8ejkmcpqxot1";
const K64: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";
const K40: &str = "abcdefghijklmnopqrstuvwxyz0123456789ABCD";
/// `random` = `000102…1f`: the private server key, its seed and `z32(public_key)`.
const RANDOM_HEX: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
const RANDOM_SEED: &str = "6c86c6aac5fb24bcf5d9939cb7d7d5645ce39418f449e03b262dd4fa14b4b92b";
const RANDOM_PUBLIC_Z32: &str = "3whp691jedt8p49a4xjc75piox4gwk9mftuxncpmwssukodx614o";
/// A public server without a key with the same `random`.
const RANDOM_PUBLIC_KEY: &str = "yqooxx9u3aemh8mo5wcqq16yufu6jitouq1o4za751dger1igghy";
const ABC_SEED: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
const ABC_PUBLIC_Z32: &str = "bxxinyn6mjjf31p9ssxtobak64wkcyjty3jo7qrweikkmm1c8u7y";
const K64_SEED: &str = "79175e70eb2236876b0c003be58294690c0e36b44c0947ae80f599ea9d039833";

mod private_server {
    use super::*;

    #[tokio::test]
    async fn info_is_the_private_connection_string() {
        let kit = kit();
        let node = started(&kit, server_opts(true, None)).await;
        let expected = HolesailInfo {
            kind: "server",
            state: Some("listening"),
            secure: true,
            port: JsValue::Number(3000.0),
            host: text("127.0.0.1"),
            protocol: "tcp",
            seed: Some(RANDOM_SEED.to_string()),
            key: RANDOM_HEX.to_string(),
            url: format!("hs://s000{RANDOM_HEX}"),
            public_key: RANDOM_PUBLIC_Z32.to_string(),
        };
        assert_eq!(node.holesail.info(), expected);
    }

    #[tokio::test]
    async fn dht_calls_match_live_private() {
        let kit = kit();
        started(&kit, server_opts(true, None)).await;
        assert_eq!(kit.factory.trace().events(), js_trace("live-private"));
    }
}

mod public_server {
    use super::*;

    #[tokio::test]
    async fn info_is_the_public_connection_string() {
        let kit = kit();
        let info = started(&kit, server_opts(false, None))
            .await
            .holesail
            .info();
        assert_eq!(
            (info.kind, info.secure, info.protocol),
            ("server", false, "tcp")
        );
        assert_eq!(info.key, RANDOM_PUBLIC_KEY);
        assert_eq!(info.public_key, RANDOM_PUBLIC_KEY);
        assert_eq!(info.url, format!("hs://0000{RANDOM_PUBLIC_KEY}"));
    }

    #[tokio::test]
    async fn dht_calls_match_live_public() {
        let kit = kit();
        started(&kit, server_opts(false, None)).await;
        assert_eq!(kit.factory.trace().events(), js_trace("live-public"));
    }
}

// `--key`: the seed is sha256 of the key string; private mode shows the key itself, public — the z32 of the public key.
mod server_with_key {
    use super::*;

    #[tokio::test]
    async fn private_key_is_shown_as_given() {
        let kit = kit();
        let info = started(&kit, server_opts(true, Some(text("abc"))))
            .await
            .holesail
            .info();
        assert_eq!(
            (info.key.as_str(), info.url.as_str()),
            ("abc", "hs://s000abc")
        );
        assert_eq!(info.seed.as_deref(), Some(ABC_SEED));
    }

    #[tokio::test]
    async fn public_mode_shows_z32_of_the_public_key_instead_of_the_key() {
        let kit = kit();
        let info = started(&kit, server_opts(false, Some(text("abc"))))
            .await
            .holesail
            .info();
        assert_eq!(info.key, ABC_PUBLIC_Z32);
        assert_eq!(info.url, format!("hs://0000{ABC_PUBLIC_Z32}"));
        assert_eq!(
            info.seed.as_deref(),
            Some(ABC_SEED),
            "the seed in public mode is sha256(key) too"
        );
    }

    #[tokio::test]
    async fn numeric_key_is_used_as_its_decimal_string() {
        let kit = kit();
        let info = started(&kit, server_opts(true, Some(JsValue::Number(123456.0))))
            .await
            .holesail
            .info();
        assert_eq!(
            (info.key.as_str(), info.url.as_str()),
            ("123456", "hs://s000123456")
        );
        let seed = "8d969eef6ecad3c29a3a629280e686cf0c3f5d5a86aff3ca12020c923adc6c92";
        assert_eq!(info.seed.as_deref(), Some(seed));
    }

    #[tokio::test]
    async fn q2_s000_url_beats_the_public_flag() {
        let kit = kit();
        let key = text(&format!("hs://s000{K40}"));
        let info = started(&kit, server_opts(false, Some(key)))
            .await
            .holesail
            .info();
        assert!(info.secure, "hs://s000… makes the server private");
        assert_eq!(
            (info.key.as_str(), info.url),
            (K40, format!("hs://s000{K40}"))
        );
    }

    #[tokio::test]
    async fn q10_zero_url_leaves_the_server_private() {
        let kit = kit();
        let key = text(&format!("hs://0000{K40}"));
        let info = started(&kit, server_opts(true, Some(key)))
            .await
            .holesail
            .info();
        assert!(info.secure, "hs://0000… does not cancel private mode");
        assert_eq!(
            (info.key.as_str(), info.url),
            (K40, format!("hs://s000{K40}"))
        );
    }
}

mod client {
    use super::*;

    #[tokio::test]
    async fn q1_without_public_flag_the_client_is_public() {
        let kit = kit();
        let info = started(&kit, client_opts(false, Z52)).await.holesail.info();
        assert_eq!(
            (info.kind, info.secure, info.protocol),
            ("client", false, "tcp")
        );
        assert_eq!(
            (info.key, info.url),
            (Z52.to_string(), format!("hs://0000{Z52}"))
        );
        assert_eq!(info.seed, None, "a public client has no seed");
    }

    #[tokio::test]
    async fn q1_public_flag_makes_the_client_private() {
        let kit = kit();
        let info = started(&kit, client_opts(true, K64)).await.holesail.info();
        assert!(info.secure, "--public on a client is private mode");
        assert_eq!(
            (info.key.as_str(), info.url),
            (K64, format!("hs://s000{K64}"))
        );
        assert_eq!(info.seed.as_deref(), Some(K64_SEED), "sha256 of the key");
    }

    #[tokio::test]
    async fn private_client_has_the_public_key_of_the_server_with_the_same_key() {
        let server = started(&kit(), server_opts(true, Some(text(K64)))).await;
        let client = started(&kit(), client_opts(true, K64)).await;
        assert_eq!(
            client.holesail.info().public_key,
            server.holesail.info().public_key
        );
    }

    #[tokio::test]
    async fn q2_sixth_char_s_makes_the_client_private() {
        let kit = kit();
        let info = started(&kit, client_opts(false, Z52_SIXTH_S))
            .await
            .holesail
            .info();
        assert!(info.secure, "the 6th character `s` enables private mode");
        assert_eq!(info.url, format!("hs://s000{Z52_SIXTH_S}"));
    }

    // JS (testvectors/probes-js/ready-badkey.cjs): `new Holesail({ client, key: 'not!valid' })` does not throw — `z32.decode` calls `new HolesailClient`
    // from `_open`, i.e. from `ready()`, which is rejected with `Invalid character in base32 input: "!" at position 3`; there is no DHT node yet.
    #[tokio::test]
    async fn q1_key_that_is_not_z32_fails_in_ready_before_any_dht_call() {
        let kit = kit();
        let mut node = build(&kit, client_opts(false, "not!valid")).expect("new does not throw");
        let error = node
            .holesail
            .ready()
            .await
            .expect_err("ready accepted a non-z32 key");
        assert_eq!(
            error.to_string(),
            "Invalid character in base32 input: \"!\" at position 3"
        );
        assert!(
            kit.factory.trace().events().is_empty(),
            "the node was created before the key was parsed"
        );
    }

    #[tokio::test]
    async fn q11_listens_on_the_record_host_and_port() {
        let port = free_port();
        let record = format!(r#"{{"host":"127.0.0.1","port":{port},"udp":false}}"#);
        let config = MockConfig::default()
            .lenient_connect(true)
            .record(RecordFor::Any, record);
        let kit = kit_with(MockFactory::with_config(config));
        let opts = HolesailOpts {
            port: None,
            host: None,
            ..client_opts(false, Z52)
        };
        let node = started(&kit, opts).await;
        let info = node.holesail.info();
        assert_eq!(
            (info.host, info.port),
            (text("127.0.0.1"), JsValue::Number(f64::from(port)))
        );
        let reached = timeout(WAIT, TcpStream::connect(("127.0.0.1", port))).await;
        reached
            .expect("connect hung")
            .expect("nobody listens at the address from the record");
    }
}

struct Kit {
    factory: MockFactory,
    stdout: Capture,
    stderr: Capture,
}

/// The facade and a `Fatal` receiver that must be kept alive.
struct Node {
    holesail: Holesail<MockFactory>,
    _fatal: UnboundedReceiver<String>,
}

fn kit() -> Kit {
    kit_with(MockFactory::new())
}

fn kit_with(factory: MockFactory) -> Kit {
    Kit {
        factory,
        stdout: Capture::new(),
        stderr: Capture::new(),
    }
}

/// Logs of all levels (`DEBUG` and above), stdout and stderr together.
fn logs(kit: &Kit) -> String {
    format!("{}{}", kit.stdout.text(), kit.stderr.text())
}

fn build(kit: &Kit, opts: HolesailOpts) -> Result<Node, HolesailError> {
    let logger = Logger::new(Some(0.0), kit.stdout.shared(), kit.stderr.shared());
    let (fatal, receiver) = Fatal::channel();
    let mut random = fixed_random();
    let holesail = Holesail::new(kit.factory.clone(), opts, logger, fatal, &mut random)?;
    Ok(Node {
        holesail,
        _fatal: receiver,
    })
}

/// `new`, `ready` and waiting for the `listening` state (`ready` returns earlier).
async fn started(kit: &Kit, opts: HolesailOpts) -> Node {
    let mut node = build(kit, opts).expect("new");
    node.holesail.ready().await.expect("ready");
    until("state = listening", || {
        node.holesail.info().state == Some("listening")
    })
    .await;
    node
}

async fn until(what: &str, mut done: impl FnMut() -> bool) {
    let poll = async {
        while !done() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    timeout(WAIT, poll)
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for: {what}"));
}

fn text(value: &str) -> JsValue {
    JsValue::String(value.to_string())
}

fn server_opts(secure: bool, key: Option<JsValue>) -> HolesailOpts {
    HolesailOpts {
        server: true,
        port: Some(JsValue::Number(3000.0)),
        host: Some(text("127.0.0.1")),
        key,
        secure,
        ..HolesailOpts::default()
    }
}

fn client_opts(secure: bool, key: &str) -> HolesailOpts {
    HolesailOpts {
        server: false,
        port: Some(JsValue::Number(0.0)),
        host: Some(text("127.0.0.1")),
        key: Some(text(key)),
        secure,
        ..HolesailOpts::default()
    }
}

/// The JS matrix case trace without `HyperDHT.keyPair` and `randomBytes`, which the mock does not record.
fn js_trace(id: &str) -> Vec<Value> {
    let matrix = vectors::load_value("holesail-cli/cli-matrix.json");
    let cases = matrix.as_array().expect("the matrix is an array");
    let case = cases
        .iter()
        .find(|case| case["id"] == id)
        .expect("no such case");
    strip_untracked(case["trace"].as_array().expect("no trace"))
}

fn free_port() -> u16 {
    let socket = TcpListener::bind("127.0.0.1:0").expect("bind");
    socket.local_addr().expect("local_addr").port()
}
