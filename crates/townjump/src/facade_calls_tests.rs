// Tests of `facade`: pause, resume, close (state, `dht.*` in the trace, log lines) and `Holesail::lookup`.
// Helpers (`kit`, `started`, `server_opts`, …) are at the bottom of facade_tests.rs.
use super::*;

const PUBLIC_RECORD: &str = r#"{"host":"10.1.2.3","port":4000,"udp":false}"#;
const UDP_RECORD: &str = r#"{"host":"0.0.0.0","port":4100,"udp":true}"#;

mod server {
    use super::*;

    #[tokio::test]
    async fn pause_suspends_the_dht_and_reports_paused() {
        let kit = kit();
        let mut node = started(&kit, server_opts(true, None)).await;
        node.holesail.pause().await.expect("pause");
        assert_eq!(node.holesail.info().state, Some("paused"));
        assert_eq!(kit.factory.trace().named("dht.suspend").len(), 1);
        assert_in_order(&logs(&kit), &["Pausing server", "Server paused"]);
    }

    #[tokio::test]
    async fn resume_resumes_the_dht_and_reports_listening() {
        let kit = kit();
        let mut node = started(&kit, server_opts(true, None)).await;
        node.holesail.pause().await.expect("pause");
        node.holesail.resume().await.expect("resume");
        assert_eq!(node.holesail.info().state, Some("listening"));
        assert_eq!(kit.factory.trace().named("dht.resume").len(), 1);
        assert_in_order(&logs(&kit), &["Resuming server", "Server resumed"]);
    }

    #[tokio::test]
    async fn close_destroys_the_dht_and_reports_destroyed() {
        let kit = kit();
        let mut node = started(&kit, server_opts(true, None)).await;
        node.holesail.close().await.expect("close");
        assert_eq!(node.holesail.info().state, Some("destroyed"));
        assert_eq!(kit.factory.trace().named("dht.destroy").len(), 1);
    }
}

mod client {
    use super::*;

    #[tokio::test]
    async fn pause_suspends_the_dht_and_reports_paused() {
        let kit = kit();
        let mut node = started(&kit, client_opts(false, Z52)).await;
        node.holesail.pause().await.expect("pause");
        assert_eq!(node.holesail.info().state, Some("paused"));
        assert_eq!(kit.factory.trace().named("dht.suspend").len(), 1);
        assert_in_order(&logs(&kit), &["Pausing client", "Client paused"]);
    }

    #[tokio::test]
    async fn resume_resumes_the_dht_and_reports_listening() {
        let kit = kit();
        let mut node = started(&kit, client_opts(false, Z52)).await;
        node.holesail.pause().await.expect("pause");
        node.holesail.resume().await.expect("resume");
        assert_eq!(node.holesail.info().state, Some("listening"));
        assert_eq!(kit.factory.trace().named("dht.resume").len(), 1);
        assert_in_order(&logs(&kit), &["Resuming client"]);
    }

    #[tokio::test]
    async fn close_destroys_the_dht_and_reports_destroyed() {
        let kit = kit();
        let mut node = started(&kit, client_opts(false, Z52)).await;
        node.holesail.close().await.expect("close");
        assert_eq!(node.holesail.info().state, Some("destroyed"));
        assert_eq!(kit.factory.trace().named("dht.destroy").len(), 1);
    }
}

// `lookup`: `{...ping(key) || {}, secure}`; the DHT node is always closed. The error text in the CLI is covered in cli_matrix.
mod lookup {
    use super::*;

    #[tokio::test]
    async fn public_record_gives_host_port_and_tcp() {
        let found = lookup_in(with_record(PUBLIC_RECORD), Z52).await;
        assert_eq!(field(&found, "host"), Some(&text("10.1.2.3")));
        assert_eq!(field(&found, "port"), Some(&JsValue::Number(4000.0)));
        assert_eq!(field(&found, "protocol"), Some(&text("tcp")));
        assert_eq!(
            field(&found, "secure"),
            None,
            "public key: secure is not set"
        );
    }

    #[tokio::test]
    async fn udp_record_gives_the_udp_protocol() {
        let found = lookup_in(with_record(UDP_RECORD), Z52).await;
        assert_eq!(field(&found, "host"), Some(&text("0.0.0.0")));
        assert_eq!(field(&found, "protocol"), Some(&text("udp")));
    }

    #[tokio::test]
    async fn s000_url_marks_the_result_secure() {
        let found = lookup_in(with_record(PUBLIC_RECORD), &format!("hs://s000{K64}")).await;
        assert_eq!(field(&found, "secure"), Some(&JsValue::Bool(true)));
        assert_eq!(field(&found, "host"), Some(&text("10.1.2.3")));
    }

    #[tokio::test]
    async fn empty_record_gives_only_the_tcp_protocol() {
        let found = lookup_in(with_record("{}"), Z52).await;
        assert_eq!(field(&found, "protocol"), Some(&text("tcp")));
        assert_eq!((field(&found, "host"), field(&found, "port")), (None, None));
    }

    #[tokio::test]
    async fn no_record_gives_an_empty_object_after_two_attempts() {
        let factory = MockFactory::new();
        let found = lookup_in(factory.clone(), Z52).await;
        assert_eq!(found, JsValue::Object(vec![]));
        let names = trace_names(&factory);
        assert_eq!(
            names,
            ["HyperDHT.new", "mutableGet", "mutableGet", "dht.destroy"]
        );
    }

    #[tokio::test]
    async fn found_record_stops_at_the_first_attempt_and_closes_the_node() {
        let factory = with_record(PUBLIC_RECORD);
        lookup_in(factory.clone(), Z52).await;
        assert_eq!(
            trace_names(&factory),
            ["HyperDHT.new", "mutableGet", "dht.destroy"]
        );
    }

    #[tokio::test]
    async fn key_that_is_not_z32_is_invalid_key_format() {
        let error = Holesail::<MockFactory>::lookup(&MockFactory::new(), "not!valid").await;
        let message = error.expect_err("lookup took a non-z32 key").to_string();
        assert_eq!(message, "Invalid key format: not!valid");
    }
}

fn with_record(json: &str) -> MockFactory {
    let config = MockConfig::default().record(RecordFor::Any, json);
    MockFactory::with_config(config)
}

async fn lookup_in(factory: MockFactory, url: &str) -> JsValue {
    let found = Holesail::<MockFactory>::lookup(&factory, url).await;
    found.expect("lookup")
}

fn trace_names(factory: &MockFactory) -> Vec<String> {
    let events = strip_untracked(&factory.trace().events());
    events
        .iter()
        .map(|event| event["ev"].as_str().expect("ev").to_string())
        .collect()
}

fn field<'a>(value: &'a JsValue, name: &str) -> Option<&'a JsValue> {
    let JsValue::Object(entries) = value else {
        panic!("the lookup result is not an object: {value:?}");
    };
    entries
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, found)| found)
}

/// Log lines go in this order (others may appear between them).
fn assert_in_order(log: &str, lines: &[&str]) {
    let mut from = 0;
    for line in lines {
        let at = log[from..].find(line);
        let at =
            at.unwrap_or_else(|| panic!("no line {line:?} after the position {from}; log: {log}"));
        from += at + line.len();
    }
}
