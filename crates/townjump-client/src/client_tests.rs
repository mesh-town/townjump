// Tests for the `HolesailClient::new` constructor, `get()` and `info()` (client.md: «The `HolesailClient` constructor», «The rest of the API»).
// In-memory mock network; shared helpers are in `crate::support`, `info_of` is at the bottom.
use crate::ClientInfo;
use crate::support::*;
use serde_json::json;
use townjump_dht::MutableRecord;

mod constructor {
    use super::*;

    #[tokio::test]
    async fn public_key_creates_an_anonymous_node() {
        let f = setup(Opts::default()).await;
        assert_eq!(f.trace(), vec![ev_new(None)]);
    }

    #[tokio::test]
    async fn private_key_creates_the_node_with_the_derived_pair() {
        let f = setup(Opts::private()).await;
        assert_eq!(f.trace(), vec![ev_new(Some(PRIVATE_PK))]);
    }
}

mod get {
    use super::*;

    #[tokio::test]
    async fn no_record_gives_none_after_one_read_with_empty_opts() {
        let f = setup(Opts::default()).await;
        let got = within("get", f.client.get()).await.expect("get");
        assert_eq!(got, None);
        assert_eq!(f.named("mutableGet"), vec![ev_get(PUBLIC_PK, json!({}))]);
    }

    #[tokio::test]
    async fn found_record_gives_seq_and_bytes() {
        let f = setup(Opts::record(r#"{"a":1}"#)).await;
        let got = within("get", f.client.get()).await.expect("get");
        assert_eq!(
            got,
            Some(MutableRecord {
                seq: 3,
                value: br#"{"a":1}"#.to_vec()
            })
        );
    }

    #[tokio::test]
    async fn private_client_reads_under_the_derived_public_key() {
        let f = setup(Opts::private()).await;
        within("get", f.client.get()).await.expect("get");
        assert_eq!(f.named("mutableGet"), vec![ev_get(PRIVATE_PK, json!({}))]);
    }
}

mod info {
    use super::*;

    #[tokio::test]
    async fn public_tcp_client_reports_the_server_key_and_its_public_key() {
        let mut f = setup(Opts::default()).await;
        let port = f.start().await;
        assert_eq!(f.info(), info_of(Mode::Public, port, "tcp"));
    }

    #[tokio::test]
    async fn private_client_reports_the_seed_as_key_and_the_derived_public_key() {
        let mut f = setup(Opts::private()).await;
        let port = f.start().await;
        assert_eq!(f.info(), info_of(Mode::Private, port, "tcp"));
    }

    #[tokio::test]
    async fn udp_client_reports_the_udp_protocol() {
        let mut f = setup(Opts::default()).await;
        let port = f.start_udp().await;
        assert_eq!(f.info(), info_of(Mode::Public, port, "udp"));
    }

    #[tokio::test]
    async fn state_is_none_before_connect() {
        // JS (testvectors/probes-js/client.cjs, `infoBeforeConnect`): TypeError "Cannot read properties of undefined (reading 'port')"; unreachable from the CLI
        // - `info` is read after `ready`. In Rust `info()` has no `Result`: before `connect` the state is `None`.
        let f = setup(Opts::default()).await;
        assert_eq!(f.info().state, None);
    }
}

fn info_of(mode: Mode, port: u16, protocol: &'static str) -> ClientInfo {
    ClientInfo {
        kind: "client",
        state: Some("listening"),
        secure: mode.secure(),
        port: num(f64::from(port)),
        host: text("127.0.0.1"),
        protocol,
        key: mode.key().to_owned(),
        public_key: match mode {
            Mode::Public => PUBLIC_INFO_KEY,
            Mode::Private => PRIVATE_INFO_KEY,
        }
        .to_owned(),
    }
}
