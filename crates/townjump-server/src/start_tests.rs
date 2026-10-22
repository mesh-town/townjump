// Tests for `start`: order of DHT calls (MX trace), return after the first `put`, modes, log lines.
// Source: server.md «`start(args)` — order of actions», «Server log messages (for `--log`)»; cli-matrix.json, `live-*` cases.
use super::fixture::*;
use serde_json::{Value, json};
use tokio::time::timeout;
use townjump_jsval::JsValue;
use townjump_testkit::mock::{MockConfig, strip_untracked};
use townjump_testkit::vectors;

const RECORD: &str = r#"{"host":"127.0.0.1","port":3000}"#;

mod new {
    use super::*;

    #[tokio::test]
    async fn creates_an_anonymous_node_before_start() {
        let fx = setup(Setup::default()).await;
        let expected = json!({ "ev": "HyperDHT.new", "keyPair": null, "opts": [] });
        assert_eq!(fx.trace.events(), vec![expected]);
    }
}

// The DHT call trace matches the JS matrix trace (without `HyperDHT.keyPair` and `randomBytes`, which the mock does not write).
mod trace {
    use super::*;

    #[tokio::test]
    async fn private_tcp_matches_live_private() {
        matches_matrix("live-private", Setup::default()).await;
    }

    #[tokio::test]
    async fn public_tcp_matches_live_public() {
        let public = Setup {
            secure: false,
            ..Setup::default()
        };
        matches_matrix("live-public", public).await;
    }

    #[tokio::test]
    async fn private_udp_matches_live_udp() {
        let udp = Setup {
            udp: Some(JsValue::Bool(true)),
            ..Setup::default()
        };
        matches_matrix("live-udp", udp).await;
    }

    #[tokio::test]
    async fn custom_host_matches_live_host() {
        let host = Setup {
            host: JsValue::String("0.0.0.0".to_string()),
            ..Setup::default()
        };
        matches_matrix("live-host", host).await;
    }

    #[tokio::test]
    async fn q5_fractional_port_matches_live_float() {
        let port = Setup {
            port: JsValue::Number(3000.5),
            ..Setup::default()
        };
        matches_matrix("live-float", port).await;
    }

    #[tokio::test]
    async fn q5_hex_port_matches_live_hex() {
        let port = Setup {
            port: JsValue::Number(16.0),
            ..Setup::default()
        };
        matches_matrix("live-hex", port).await;
    }

    #[tokio::test]
    async fn q13_port_without_range_check_matches_live_big_port() {
        let port = Setup {
            port: JsValue::Number(99999.0),
            ..Setup::default()
        };
        matches_matrix("live-big-port", port).await;
    }
}

// `start` returns after the first `put`, not after `listen` (`listen` runs without `await`).
mod q8_returns_after_first_put {
    use super::*;

    #[tokio::test]
    async fn without_waiting_for_listen() {
        let mut fx = setup(hold_listen()).await;
        let args = fx.args.clone();
        let started = timeout(WAIT, fx.server.start(args)).await;
        assert!(started.expect("start waits for listen").is_ok());
        assert_eq!(fx.server.info().state, None, "listen has not finished yet");
        assert!(
            !listening_logged(&fx),
            "the listen line appeared before listen finished"
        );
    }

    #[tokio::test]
    async fn but_not_before_the_put_finishes() {
        let mut fx = setup(hold_put()).await;
        let (args, gate) = (fx.args.clone(), fx.factory.clone());
        let start = fx.server.start(args);
        tokio::pin!(start);
        let early = timeout(SILENCE, &mut start).await;
        assert!(early.is_err(), "start returned before put finished");
        gate.release_put();
        let done = timeout(WAIT, &mut start).await;
        done.expect("start did not return after put").unwrap();
    }

    #[tokio::test]
    async fn listen_is_requested_before_the_put_starts() {
        let mut fx = setup(hold_put()).await;
        let (args, trace) = (fx.args.clone(), fx.trace.clone());
        let start = fx.server.start(args);
        tokio::pin!(start);
        let _pending = timeout(SILENCE, &mut start).await;
        assert_eq!(trace.named("server.listen").len(), 1, "listen before put");
        assert!(trace.named("mutablePut").is_empty(), "put waits for gate");
    }

    #[tokio::test]
    async fn listening_state_and_line_follow_listen_completion() {
        let mut fx = setup(hold_listen()).await;
        fx.start().await;
        fx.factory.release_listen();
        fx.wait_listening().await;
        fx.wait_log("INFO", &private_listening_line()).await;
    }
}

mod mode {
    use super::*;

    #[tokio::test]
    async fn private_says_using_private_mode() {
        let fx = Fixture::started(Setup::default()).await;
        assert!(fx.has("INFO", "Using Private Mode"));
        assert!(!fx.has("INFO", "Using Public Mode"));
    }

    #[tokio::test]
    async fn public_says_using_public_mode() {
        let fx = Fixture::started(Setup {
            secure: false,
            ..Setup::default()
        })
        .await;
        assert!(fx.has("INFO", "Using Public Mode"));
        assert!(!fx.has("INFO", "Using Private Mode"));
    }
}

mod log {
    use super::*;

    #[tokio::test]
    async fn private_start_writes_the_js_message_sequence() {
        let mut fx = setup(hold_listen()).await;
        fx.start().await;
        let seed_line = format!("Generated key pair from seed: {}", masked(SEED_HEX)); // secrets are masked in the log
        let expected = [
            ("INFO", "Starting server".to_string()),
            ("DEBUG", seed_line),
            ("INFO", "Using Private Mode".to_string()),
            ("DEBUG", "Server created, awaiting listen".to_string()),
            (
                "DEBUG",
                format!("Initializing DHT with host info: {RECORD}"),
            ),
            ("DEBUG", format!("Putting DHT record: {RECORD}")),
            (
                "DEBUG",
                format!("Incoming data type: string, value: {RECORD}"),
            ),
            ("DEBUG", "Checking for existing DHT record".to_string()),
            (
                "DEBUG",
                "No existing DHT record found, creating new".to_string(),
            ),
            ("DEBUG", "DHT put completed with seq: 0".to_string()),
        ];
        let expected: Vec<(String, String)> = expected
            .into_iter()
            .map(|(level, msg)| (level.to_string(), msg))
            .collect();
        assert_eq!(fx.log(), expected);
    }

    #[tokio::test]
    async fn q9_private_listening_line_shows_a_fingerprint_of_the_key() {
        let fx = Fixture::started(Setup::default()).await;
        fx.wait_log("INFO", &private_listening_line()).await;
    }

    // in public mode the key is the public key, not a secret; JS prints it as is, no mask is applied.
    #[tokio::test]
    async fn public_listening_line_shows_the_public_key() {
        let fx = Fixture::started(Setup {
            secure: false,
            ..Setup::default()
        })
        .await;
        let line = format!("Server listening on key: {PUBLIC_KEY_Z32}");
        fx.wait_log("INFO", &line).await;
    }

    // the secret is everything the key pair is built from; the seed in public mode too, hence the mask in both modes.
    #[tokio::test]
    async fn q9_seed_line_shows_a_fingerprint_in_both_modes() {
        let line = format!("Generated key pair from seed: {}", masked(SEED_HEX));
        for secure in [true, false] {
            let fx = Fixture::started(Setup {
                secure,
                ..Setup::default()
            })
            .await;
            assert!(fx.has("DEBUG", &line), "secure = {secure}: {:?}", fx.log());
        }
    }

    #[tokio::test]
    async fn q9_private_secrets_never_appear_in_the_output() {
        let fx = Fixture::started(Setup::default()).await;
        fx.wait_log("INFO", &private_listening_line()).await;
        let output = format!("{}{}", fx.out.text(), fx.err.text());
        assert!(!output.contains(SEED_HEX), "seed in hex reached the log");
        assert!(
            !output.contains(PRIVATE_KEY_Z32),
            "z32(sha256(K)) reached the log"
        );
    }

    #[tokio::test]
    async fn every_line_has_iso_stamp_prefix_and_level() {
        let fx = Fixture::started(Setup::default()).await;
        let lines = fx.log(); // `log()` panics on a line not in the format `<ISO> [Holesail] [LEVEL] msg`
        assert!(lines.len() >= 10, "only {} lines in the log", lines.len());
    }

    #[tokio::test]
    async fn happy_path_writes_nothing_to_stderr() {
        let fx = Fixture::started(Setup::default()).await;
        assert_eq!(fx.err.text(), "");
    }
}

fn hold_listen() -> Setup {
    Setup {
        mock: MockConfig::default().hold_listen(true),
        ..Setup::default()
    }
}

fn hold_put() -> Setup {
    Setup {
        mock: MockConfig::default().hold_put(true),
        ..Setup::default()
    }
}

fn private_listening_line() -> String {
    format!("Server listening on key: {}", masked(PRIVATE_KEY_Z32)) // secrets are masked in the log
}

fn listening_logged(fx: &Fixture) -> bool {
    let is_listening = |msg: &String| msg.starts_with("Server listening on key: ");
    fx.messages("INFO").iter().any(is_listening)
}

/// Runs `start` with `opts`, the seed from the JS trace of case `id`, and compares the mock trace with the JS trace.
async fn matches_matrix(id: &str, mut opts: Setup) {
    let matrix = vectors::load_value("holesail-cli/cli-matrix.json");
    let cases = matrix.as_array().expect("matrix is an array");
    let case = cases.iter().find(|c| c["id"] == id).expect("no such case");
    let js_trace = case["trace"].as_array().expect("no trace");
    opts.seed_hex = Some(js_seed(js_trace));
    let fx = Fixture::started(opts).await;
    assert_eq!(fx.trace.events(), strip_untracked(js_trace), "trace {id}");
}

fn js_seed(trace: &[Value]) -> String {
    let event = trace.iter().find(|e| e["ev"] == "HyperDHT.keyPair");
    let seed = event.expect("no HyperDHT.keyPair")["seed"].as_str();
    seed.expect("no seed").to_string()
}
