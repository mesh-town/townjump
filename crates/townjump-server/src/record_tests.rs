// Tests for the server DHT record: the value, the choice of `seq` in `put`, `get`.
// Source: dht-records.md, server.md «put/get»; dht-record-live.json vectors; JS holesail-server/index.js (put, get).
use super::fixture::*;
use serde_json::Value;
use townjump_dht::Dht;
use townjump_jsval::JsValue;
use townjump_testkit::key_pair;
use townjump_testkit::vectors;

const OLD: &str = r#"{"host":"127.0.0.1","port":3000}"#;
const NEW: &str = r#"{"host":"127.0.0.1","port":3001}"#;

// Step-by-step replay of the vector cases: `start`, then `put` of each step and a `get` read.
mod live_vectors {
    use super::*;

    #[tokio::test]
    async fn private_tcp() {
        replay("private tcp key=abc").await;
    }

    #[tokio::test]
    async fn private_udp() {
        replay("private udp").await;
    }

    #[tokio::test]
    async fn public_tcp() {
        replay("public tcp (seed = sha256(key))").await;
    }

    #[tokio::test]
    async fn q6_public_udp_string_false() {
        replay("public udp string false").await;
    }
}

// Record value is `JSON.stringify({host, udp, port})`: key order, no type coercion.
mod value_shape {
    use super::*;

    #[tokio::test]
    async fn tcp_has_no_udp_key() {
        assert_eq!(record_for(Setup::default()).await, OLD);
    }

    #[tokio::test]
    async fn udp_true_sits_between_host_and_port() {
        let expected = r#"{"host":"127.0.0.1","udp":true,"port":3000}"#;
        assert_eq!(udp_record(JsValue::Bool(true)).await, expected);
    }

    #[tokio::test]
    async fn q6_udp_string_false_stays_a_string() {
        let expected = r#"{"host":"127.0.0.1","udp":"false","port":3000}"#;
        assert_eq!(
            udp_record(JsValue::String("false".to_string())).await,
            expected
        );
    }

    #[tokio::test]
    async fn udp_false_is_kept_not_dropped() {
        let expected = r#"{"host":"127.0.0.1","udp":false,"port":3000}"#;
        assert_eq!(udp_record(JsValue::Bool(false)).await, expected);
    }

    #[tokio::test]
    async fn udp_zero_is_kept_as_a_number() {
        let expected = r#"{"host":"127.0.0.1","udp":0,"port":3000}"#;
        assert_eq!(udp_record(JsValue::Number(0.0)).await, expected);
    }

    #[tokio::test]
    async fn q5_fractional_port_is_written_as_is() {
        let expected = r#"{"host":"127.0.0.1","port":3000.5}"#;
        assert_eq!(port_record(JsValue::Number(3000.5)).await, expected);
    }

    #[tokio::test]
    async fn q5_hex_port_is_the_decimal_number_minimist_made() {
        let expected = r#"{"host":"127.0.0.1","port":16}"#;
        assert_eq!(port_record(JsValue::Number(16.0)).await, expected);
    }

    #[tokio::test]
    async fn q13_port_above_65535_is_not_checked() {
        let expected = r#"{"host":"127.0.0.1","port":99999}"#;
        assert_eq!(port_record(JsValue::Number(99999.0)).await, expected);
    }

    #[tokio::test]
    async fn q13_zero_and_negative_ports_are_not_checked() {
        assert_eq!(
            port_record(JsValue::Number(0.0)).await,
            r#"{"host":"127.0.0.1","port":0}"#
        );
        assert_eq!(
            port_record(JsValue::Number(-1.0)).await,
            r#"{"host":"127.0.0.1","port":-1}"#
        );
    }

    #[tokio::test]
    async fn q5_nan_port_becomes_null() {
        let expected = r#"{"host":"127.0.0.1","port":null}"#;
        assert_eq!(port_record(JsValue::Number(f64::NAN)).await, expected);
    }

    #[tokio::test]
    async fn string_port_stays_a_string() {
        let expected = r#"{"host":"127.0.0.1","port":"0x10"}"#;
        assert_eq!(
            port_record(JsValue::String("0x10".to_string())).await,
            expected
        );
    }

    #[tokio::test]
    async fn empty_host_is_written_as_is() {
        assert_eq!(host_record("").await, r#"{"host":"","port":3000}"#);
    }

    #[tokio::test]
    async fn host_is_json_escaped_but_not_ascii_escaped() {
        assert_eq!(host_record("a\"b").await, r#"{"host":"a\"b","port":3000}"#);
        assert_eq!(host_record("χοστ").await, r#"{"host":"χοστ","port":3000}"#);
    }
}

// `put`: `seq` is not set if there is no record; the old `seq` for equal bytes; `seq + 1` for different ones.
mod put {
    use super::*;

    mod no_record {
        use super::*;

        #[tokio::test]
        async fn first_put_has_no_seq_and_completes_with_zero() {
            let fx = Fixture::started(Setup::default()).await;
            let put = fx.trace.named("mutablePut");
            assert_eq!(put.len(), 1);
            assert!(put[0].get("seq").is_none(), "seq is set: {}", put[0]);
            assert_eq!(put[0]["optKeys"], serde_json::json!([]));
            assert!(fx.has("DEBUG", "No existing DHT record found, creating new"));
            assert!(fx.has("DEBUG", "DHT put completed with seq: 0"));
        }
    }

    mod same_bytes {
        use super::*;

        #[tokio::test]
        async fn keeps_the_seq() {
            let fx = Fixture::started(Setup::default()).await;
            assert_eq!(fx.server.put(OLD).await.unwrap(), 0);
            assert_eq!(seq_of_last_put(&fx), Some(0));
        }

        #[tokio::test]
        async fn logs_the_js_sequence() {
            let fx = Fixture::started(Setup::default()).await;
            let before = fx.log().len();
            fx.server.put(OLD).await.unwrap();
            assert_eq!(fx.log()[before..], put_lines(OLD, OLD, 0, true, 0));
        }
    }

    mod different_bytes {
        use super::*;

        #[tokio::test]
        async fn increments_the_seq() {
            let fx = Fixture::started(Setup::default()).await;
            assert_eq!(fx.server.put(NEW).await.unwrap(), 1);
            assert_eq!(seq_of_last_put(&fx), Some(1));
            assert_eq!(fx.server.put(OLD).await.unwrap(), 2);
        }

        #[tokio::test]
        async fn logs_the_js_sequence() {
            let fx = Fixture::started(Setup::default()).await;
            let before = fx.log().len();
            fx.server.put(NEW).await.unwrap();
            assert_eq!(fx.log()[before..], put_lines(NEW, OLD, 0, false, 1));
        }
    }

    mod record_already_in_the_network {
        use super::*;

        #[tokio::test]
        async fn different_value_goes_above_the_highest_seq() {
            let mut fx = setup(Setup::default()).await;
            plant(&fx, 2, "old").await;
            plant(&fx, 5, "newer").await;
            fx.start().await;
            assert_eq!(seq_of_last_put(&fx), Some(6));
            assert_eq!(fx.server.get().await.unwrap().unwrap().seq, 6);
        }

        #[tokio::test]
        async fn same_value_republishes_the_highest_seq() {
            let mut fx = setup(Setup::default()).await;
            plant(&fx, 4, OLD).await;
            fx.start().await;
            assert_eq!(seq_of_last_put(&fx), Some(4));
        }

        #[tokio::test]
        async fn put_asks_for_the_latest_record() {
            let fx = Fixture::started(Setup::default()).await;
            let gets = fx.trace.named("mutableGet");
            assert_eq!(gets[0]["opts"], serde_json::json!({ "latest": true }));
        }
    }
}

mod get {
    use super::*;

    #[tokio::test]
    async fn returns_the_record_the_server_put() {
        let fx = Fixture::started(Setup::default()).await;
        let record = fx.server.get().await.unwrap().expect("no record");
        assert_eq!((record.seq, record.value), (0, OLD.as_bytes().to_vec()));
    }

    #[tokio::test]
    async fn returns_the_newest_after_a_put() {
        let fx = Fixture::started(Setup::default()).await;
        fx.server.put(NEW).await.unwrap();
        let record = fx.server.get().await.unwrap().expect("no record");
        assert_eq!((record.seq, record.value), (1, NEW.as_bytes().to_vec()));
    }

    #[tokio::test]
    async fn logs_the_record_it_found() {
        let fx = Fixture::started(Setup::default()).await;
        let before = fx.log().len();
        fx.server.get().await.unwrap();
        let text = format!("Existing DHT record found: seq=0, value={OLD}");
        assert_eq!(fx.log()[before..], [("DEBUG".to_string(), text)]);
    }
}

/// Vector `dht-record-live.json`: case `label`.
async fn replay(label: &str) {
    let all = vectors::load_value("holesail/dht-record-live.json");
    let cases = all["cases"].as_array().expect("cases");
    let case = cases
        .iter()
        .find(|c| c["label"] == label)
        .expect("no such case");
    let mut fx = setup(setup_of(&case["args"])).await;
    fx.start().await;
    for step in case["steps"].as_array().expect("steps") {
        let name = step["step"].as_str().unwrap_or_default();
        if let Some(text) = step["putValueText"].as_str() {
            let seq = fx
                .server
                .put(text)
                .await
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(
                Some(seq),
                step["putReturnedSeq"].as_u64(),
                "{name}: seq put"
            );
        }
        let record = fx.server.get().await.unwrap().expect("no record");
        assert_eq!(
            Some(record.seq),
            step["read"]["seq"].as_u64(),
            "{name}: record seq"
        );
        let text = String::from_utf8(record.value).expect("utf-8");
        assert_eq!(
            Some(text.as_str()),
            step["read"]["valueText"].as_str(),
            "{name}: value"
        );
    }
}

/// Arguments of a vector case (`udp`: `"undefined"` means no key).
fn setup_of(args: &Value) -> Setup {
    let udp = match &args["udp"] {
        Value::String(s) if s == "undefined" => None,
        Value::String(s) => Some(JsValue::String(s.clone())),
        other => Some(JsValue::Bool(other.as_bool().expect("udp"))),
    };
    Setup {
        secure: args["secure"].as_bool().expect("secure"),
        port: JsValue::Number(args["port"].as_f64().expect("port")),
        host: JsValue::String(args["host"].as_str().expect("host").to_string()),
        udp,
        seed_hex: args["seedHex"].as_str().map(str::to_string),
        ..Setup::default()
    }
}

/// Value text of the first `mutablePut` after `start`.
async fn record_for(opts: Setup) -> String {
    Fixture::started(opts).await.put_value()
}

async fn udp_record(udp: JsValue) -> String {
    record_for(Setup {
        udp: Some(udp),
        ..Setup::default()
    })
    .await
}

async fn port_record(port: JsValue) -> String {
    record_for(Setup {
        port,
        ..Setup::default()
    })
    .await
}

async fn host_record(host: &str) -> String {
    record_for(Setup {
        host: JsValue::String(host.to_string()),
        ..Setup::default()
    })
    .await
}

/// The `seq` field of the last `mutablePut` in the trace.
fn seq_of_last_put(fx: &Fixture) -> Option<u64> {
    let puts = fx.trace.named("mutablePut");
    puts.last().expect("no mutablePut")["seq"].as_u64()
}

/// Record `text` with version `seq` under the server key, from another network node (before `start`).
async fn plant(fx: &Fixture, seq: u64, text: &str) {
    let node = fx.client(key_pair(9)).await;
    let put = node
        .mutable_put(&server_pair(), text.as_bytes(), Some(seq))
        .await;
    put.expect("record preparation");
}

/// DEBUG lines of `put(new)` over record `old` with version `old_seq`; final version `seq`.
fn put_lines(new: &str, old: &str, old_seq: u64, same: bool, seq: u64) -> Vec<(String, String)> {
    let lines = [
        format!("Putting DHT record: {new}"),
        format!("Incoming data type: string, value: {new}"),
        "Checking for existing DHT record".to_string(),
        format!("Existing DHT record found: seq={old_seq}, value={old}"),
        format!("Existing record found, putting with seq: {seq} (same: {same})"),
        format!("DHT put completed with seq: {seq}"),
    ];
    lines
        .into_iter()
        .map(|msg| ("DEBUG".to_string(), msg))
        .collect()
}
