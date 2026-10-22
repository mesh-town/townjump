// Tests for `HolesailClient::connect`: log texts and order, node trace, wrong key, bind error (client.md «connect(options)»).
// Merging flags with the record is in record_tests.rs; network is the mock and 127.0.0.1 only; shared helpers are in `crate::support`.
use crate::support::*;
use crate::{ClientError, ConnectArgs};
use std::net::TcpListener;

mod log_order {
    use super::*;

    #[tokio::test]
    async fn public_client_without_record_logs_the_whole_sequence() {
        let mut f = setup(Opts::default()).await;
        let port = f.start().await;
        let want = [
            format!("[INFO] Connecting to key: {PUBLIC_Z32}, secure: false"),
            "[DEBUG] Getting DHT record".to_owned(),
            "[WARN] DHT get: no record found".to_owned(),
            "[WARN] No DHT data retrieved".to_owned(),
            "[DEBUG] Handling TCP connection".to_owned(),
            format!("[INFO] Proxy listening on 127.0.0.1:{port}"),
        ];
        assert_eq!(f.lines(), want);
    }

    #[tokio::test]
    async fn found_record_logs_seq_value_and_parsed_data() {
        let port = free_port();
        let record = format!(r#"{{"host":"127.0.0.1","port":{port}}}"#);
        let mut f = setup(Opts::record(record.clone())).await;
        f.connect_ok(ConnectArgs::default()).await;
        f.wait_listening().await;
        let want = [
            format!("[INFO] Connecting to key: {PUBLIC_Z32}, secure: false"),
            "[DEBUG] Getting DHT record".to_owned(),
            format!("[DEBUG] DHT get completed: seq=3, value={record}"),
            format!("[DEBUG] Retrieved DHT data: {record}"),
            "[DEBUG] Handling TCP connection".to_owned(),
            format!("[INFO] Proxy listening on 127.0.0.1:{port}"),
        ];
        assert_eq!(f.lines(), want);
    }

    #[tokio::test]
    async fn udp_client_logs_handling_udp_and_listening_for_udp() {
        let mut f = setup(Opts::default()).await;
        let port = f.start_udp().await;
        let lines = f.lines();
        assert_eq!(lines[4], "[DEBUG] Handling UDP connection");
        assert_eq!(
            lines[5],
            format!("[INFO] Proxy listening on 127.0.0.1:{port} for UDP")
        );
    }

    #[tokio::test]
    async fn private_client_logs_a_fingerprint_instead_of_the_secret() {
        // intentional divergence from JS: first 8 characters of the secret, then one `*` per character.
        let mut f = setup(Opts::private()).await;
        f.start().await;
        let stars = "*".repeat(PRIVATE_SEED_Z32.len() - 8);
        let want = format!(
            "[INFO] Connecting to key: {}{stars}, secure: true",
            &PRIVATE_SEED_Z32[..8]
        );
        assert_eq!(f.lines()[0], want);
        let leaked = f.log.text().contains(PRIVATE_SEED_Z32);
        assert!(!leaked, "the full secret must not reach the log");
    }
}

mod dht_trace {
    use super::*;

    #[tokio::test]
    async fn public_key_gives_an_anonymous_node_and_one_read_of_the_record() {
        let mut f = setup(Opts::default()).await;
        f.start().await;
        assert_eq!(f.trace(), matrix_trace("conn-public-z32"));
    }

    #[tokio::test]
    async fn private_key_gives_the_derived_pair_for_the_node_and_the_read() {
        let key = matrix_seed_z32("conn-z32-6s");
        let mut f = setup(Opts {
            mode: Mode::Private,
            key: Some(key),
            ..Opts::default()
        })
        .await;
        f.start().await;
        assert_eq!(f.trace(), matrix_trace("conn-z32-6s"));
    }
}

mod bad_key {
    use super::*;

    #[tokio::test]
    async fn bare_hex_key_in_public_mode_is_a_keys_error() {
        // a 64-hex string without a prefix is read as a public z32 key, and it has no `0`.
        let (factory, log) = (
            townjump_testkit::mock::MockFactory::new(),
            townjump_testkit::Capture::new(),
        );
        let err = error_of(try_new(&factory, &log, PUBLIC_PK, false).await);
        assert!(
            matches!(err, ClientError::Keys(_)),
            "expected Keys, got {err:?}"
        );
        assert!(
            normalized(factory.trace().events()).is_empty(),
            "the node must not be created"
        );
    }

    #[tokio::test]
    async fn key_outside_the_alphabet_in_private_mode_is_a_keys_error() {
        // an invalid z32 is an exception, exit code 1.
        let (factory, log) = (
            townjump_testkit::mock::MockFactory::new(),
            townjump_testkit::Capture::new(),
        );
        let err = error_of(try_new(&factory, &log, "not*a*z32*key", true).await);
        assert!(
            matches!(err, ClientError::Keys(_)),
            "expected Keys, got {err:?}"
        );
    }
}

mod bind_failure {
    use super::*;

    #[tokio::test]
    async fn busy_port_reports_address_in_use_to_fatal_after_connect_returned() {
        // `connect` returns before bind; a listen error is an unhandled exception (`Fatal`).
        let holder = TcpListener::bind("127.0.0.1:0").expect("bind 0");
        let port = holder.local_addr().expect("local_addr").port();
        let mut f = setup(Opts::default()).await;
        f.connect_ok(local(port)).await;
        let message = f.next_fatal().await.to_lowercase();
        assert!(
            message.contains("address already in use"),
            "Fatal text: {message}"
        );
        let listening = f.lines().iter().any(|l| l.contains("Proxy listening"));
        assert!(
            !listening,
            "`Proxy listening` must not be printed when the port is busy"
        );
    }

    #[tokio::test]
    async fn free_port_raises_no_fatal() {
        let mut f = setup(Opts::default()).await;
        f.start().await;
        f.no_fatal().await;
    }
}
