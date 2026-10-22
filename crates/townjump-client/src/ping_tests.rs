// Tests for `ping` (`--lookup`): attempt A "as private", attempt B "as public", `protocol`, rejections (dht-records.md «--lookup»).
// `lookup-*` traces come from the matrix; records are put into the mock network via `publish`; helpers are in `crate::support`.
use crate::support::*;
use crate::{ClientError, LookupRecord, ping};
use townjump_jsval::JsValue;
use townjump_testkit::mock::{MockConfig, MockFactory, RecordFor};

mod nothing_found {
    use super::*;

    #[tokio::test]
    async fn gives_none_and_asks_twice_then_destroys_the_node() {
        let factory = MockFactory::new();
        let got = within("ping", ping(&factory, PUBLIC_Z32))
            .await
            .expect("ping");
        assert_eq!(got, None);
        assert_eq!(
            normalized(factory.trace().events()),
            matrix_trace("lookup-none")
        );
    }

    #[tokio::test]
    async fn json_error_in_every_attempt_is_skipped_and_gives_none() {
        let config = MockConfig::default().record(RecordFor::Any, "not json");
        let factory = MockFactory::with_config(config);
        let got = within("ping", ping(&factory, PUBLIC_Z32))
            .await
            .expect("ping");
        assert_eq!(got, None);
        assert_eq!(
            normalized(factory.trace().events()),
            matrix_trace("lookup-badjson")
        );
    }
}

mod found {
    use super::*;

    #[tokio::test]
    async fn record_under_the_derived_key_ends_after_the_first_attempt() {
        let factory = MockFactory::new();
        publish(
            &factory,
            DERIVED_PK,
            r#"{"host":"10.1.2.3","port":4000}"#,
            1,
        )
        .await;
        let got = within("ping", ping(&factory, PUBLIC_Z32))
            .await
            .expect("ping");
        assert_eq!(
            got,
            Some(found_record(
                &[("host", text("10.1.2.3")), ("port", num(4000.0))],
                "tcp"
            ))
        );
        assert_eq!(
            factory.trace().named("mutableGet").len(),
            1,
            "attempt B is not needed"
        );
        assert_last_is_destroy(&factory);
    }

    #[tokio::test]
    async fn record_under_the_key_itself_is_found_by_the_second_attempt() {
        let factory = MockFactory::new();
        publish(&factory, PUBLIC_PK, r#"{"host":"10.1.2.3","port":4000}"#, 1).await;
        let got = within("ping", ping(&factory, PUBLIC_Z32))
            .await
            .expect("ping");
        assert_eq!(
            got,
            Some(found_record(
                &[("host", text("10.1.2.3")), ("port", num(4000.0))],
                "tcp"
            ))
        );
        assert_eq!(factory.trace().named("mutableGet").len(), 2);
        assert_last_is_destroy(&factory);
    }

    #[tokio::test]
    async fn udp_true_in_the_record_gives_protocol_udp_after_the_record_keys() {
        let factory = MockFactory::new();
        publish(
            &factory,
            PUBLIC_PK,
            r#"{"udp":true,"host":"0.0.0.0","port":4100}"#,
            1,
        )
        .await;
        let got = within("ping", ping(&factory, PUBLIC_Z32))
            .await
            .expect("ping");
        let keys = [
            ("udp", JsValue::Bool(true)),
            ("host", text("0.0.0.0")),
            ("port", num(4100.0)),
        ];
        assert_eq!(got, Some(found_record(&keys, "udp")));
    }

    #[tokio::test]
    async fn empty_object_record_gives_just_protocol_tcp() {
        let factory = MockFactory::new();
        publish(&factory, DERIVED_PK, "{}", 1).await;
        let got = within("ping", ping(&factory, PUBLIC_Z32))
            .await
            .expect("ping");
        assert_eq!(got, Some(found_record(&[], "tcp")));
    }

    #[tokio::test]
    async fn invalid_json_in_attempt_a_falls_through_to_attempt_b() {
        let factory = MockFactory::new();
        publish(&factory, DERIVED_PK, "not json", 1).await;
        publish(&factory, PUBLIC_PK, r#"{"port":1}"#, 1).await;
        let got = within("ping", ping(&factory, PUBLIC_Z32))
            .await
            .expect("ping");
        assert_eq!(got, Some(found_record(&[("port", num(1.0))], "tcp")));
    }

    #[tokio::test]
    async fn null_json_in_attempt_a_falls_through_to_attempt_b() {
        // JS (testvectors/probes-js/client.cjs, `ping`): `null.protocol = …` is a TypeError inside `try`, the result stays null, the attempt is empty.
        let factory = MockFactory::new();
        publish(&factory, DERIVED_PK, "null", 1).await;
        publish(&factory, PUBLIC_PK, r#"{"port":1}"#, 1).await;
        let got = within("ping", ping(&factory, PUBLIC_Z32))
            .await
            .expect("ping");
        assert_eq!(got, Some(found_record(&[("port", num(1.0))], "tcp")));
    }
}

mod bad_key {
    use super::*;

    #[tokio::test]
    async fn key_outside_the_alphabet_is_a_keys_error() {
        let factory = MockFactory::new();
        let err = error_of(within("ping", ping(&factory, "0000")).await);
        assert!(
            matches!(err, ClientError::Keys(_)),
            "expected Keys, got {err:?}"
        );
    }

    #[tokio::test]
    async fn seed_that_is_not_32_bytes_is_a_keys_error() {
        // Attempt A takes the key as a seed: it needs a length of 32 bytes.
        let factory = MockFactory::new();
        let short = z32_encode(&[7u8; 16]);
        let err = error_of(within("ping", ping(&factory, &short)).await);
        assert!(
            matches!(err, ClientError::Keys(_)),
            "expected Keys, got {err:?}"
        );
    }
}

/// Record found: parsed keys in JSON order plus `protocol` appended at the end.
fn found_record(keys: &[(&str, JsValue)], protocol: &str) -> LookupRecord {
    let mut all: Vec<(&str, JsValue)> = keys.to_vec();
    all.push(("protocol", text(protocol)));
    LookupRecord { record: obj(&all) }
}

fn assert_last_is_destroy(factory: &MockFactory) {
    let events = factory.trace().events();
    let last = events.last().expect("trace is empty");
    assert_eq!(
        last,
        &ev("dht.destroy"),
        "the node is destroyed before returning"
    );
}
