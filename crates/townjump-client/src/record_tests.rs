// Tests for `connect`: merging flags with the DHT record (`??`) and an invalid record - client.md, steps 3-5.
// The result is read via `info()`; `setup` and helpers are in `crate::support`, `merged` is at the bottom.
use crate::support::*;
use crate::{ClientError, ClientInfo, ConnectArgs};
use townjump_jsval::{JsError, JsValue};

mod record_and_flags {
    use super::*;

    #[tokio::test]
    async fn flags_alone_are_used_when_there_is_no_record() {
        let port = free_port();
        let info = merged(None, local(port)).await;
        assert_eq!(
            (info.port, info.host, info.protocol),
            (num(f64::from(port)), text("127.0.0.1"), "tcp")
        );
    }

    #[tokio::test]
    async fn record_host_and_port_are_used_without_flags_and_the_port_listens() {
        // the client listens on the same host:port as the server does.
        let port = free_port();
        let mut f = setup(Opts::record(record(port, ""))).await;
        f.connect_ok(ConnectArgs::default()).await;
        f.wait_listening().await;
        let info = f.info();
        assert_eq!(
            (info.port, info.host),
            (num(f64::from(port)), text("127.0.0.1"))
        );
        let _client = tcp_connect(port).await;
    }

    #[tokio::test]
    async fn flags_win_over_the_record() {
        let (from_flags, from_record) = (free_port(), free_port());
        let json = format!(r#"{{"host":"10.1.2.3","port":{from_record}}}"#);
        let info = merged(Some(&json), local(from_flags)).await;
        assert_eq!(
            (info.port, info.host),
            (num(f64::from(from_flags)), text("127.0.0.1"))
        );
    }

    #[tokio::test]
    async fn no_flags_and_no_record_give_8989_on_loopback() {
        let info = merged(None, ConnectArgs::default()).await;
        assert_eq!((info.port, info.host), (num(8989.0), text("127.0.0.1")));
    }

    #[tokio::test]
    async fn port_zero_flag_is_kept_and_does_not_fall_to_the_record() {
        // `??` with `0`: `0` is not null, so the record yields.
        let args = ConnectArgs {
            port: Some(num(0.0)),
            host: Some(text("127.0.0.1")),
            udp: None,
        };
        let info = merged(Some(&record(free_port(), "")), args).await;
        assert_eq!(info.port, num(0.0));
    }

    #[tokio::test]
    async fn empty_host_flag_is_kept_and_does_not_fall_to_the_record() {
        let args = ConnectArgs {
            host: Some(text("")),
            ..local(free_port())
        };
        let info = merged(Some(&record(free_port(), "")), args).await;
        assert_eq!(info.host, text(""));
    }

    #[tokio::test]
    async fn null_flag_falls_through_to_the_record() {
        let port = free_port();
        let args = ConnectArgs {
            port: Some(JsValue::Null),
            host: Some(JsValue::Null),
            udp: None,
        };
        let info = merged(Some(&record(port, "")), args).await;
        assert_eq!(
            (info.port, info.host),
            (num(f64::from(port)), text("127.0.0.1"))
        );
    }
}

mod udp_flag {
    use super::*;

    #[tokio::test]
    async fn no_udp_anywhere_means_tcp() {
        let info = merged(None, local(free_port())).await;
        assert_eq!(info.protocol, "tcp");
    }

    #[tokio::test]
    async fn udp_true_in_the_record_means_udp_without_flags() {
        let info = merged(
            Some(&record(free_udp_port(), r#","udp":true"#)),
            ConnectArgs::default(),
        )
        .await;
        assert_eq!(info.protocol, "udp");
    }

    #[tokio::test]
    async fn false_flag_wins_over_udp_in_the_record() {
        let json = record(free_port(), r#","udp":true"#);
        let args = ConnectArgs {
            udp: Some(JsValue::Bool(false)),
            ..local(free_port())
        };
        assert_eq!(merged(Some(&json), args).await.protocol, "tcp");
    }

    #[tokio::test]
    async fn string_false_flag_is_truthy_and_means_udp() {
        // `--udp=false` gives the string 'false', which is truthy.
        let args = ConnectArgs {
            udp: Some(text("false")),
            ..local(free_udp_port())
        };
        assert_eq!(merged(None, args).await.protocol, "udp");
    }
}

mod bad_record {
    use super::*;

    #[tokio::test]
    async fn text_that_is_not_json_fails_connect_with_a_syntax_error() {
        // `JSON.parse` of a foreign record is an exception, exit code 1.
        let mut f = setup(Opts::record("not json")).await;
        let err = error_of(f.connect(local(free_port())).await);
        assert!(
            matches!(err, ClientError::Js(JsError::Syntax(_))),
            "expected SyntaxError: {err:?}"
        );
        let started = f
            .lines()
            .iter()
            .any(|l| l.contains("Handling TCP connection"));
        assert!(!started, "the proxy must not start");
    }

    #[tokio::test]
    async fn null_record_fails_connect_with_a_type_error() {
        // `JSON.parse('null')` -> access to a field of `null`.
        let mut f = setup(Opts::record("null")).await;
        let err = error_of(f.connect(local(free_port())).await);
        assert!(
            matches!(err, ClientError::Js(JsError::Type(_))),
            "expected TypeError: {err:?}"
        );
    }
}

/// Server record `{"host":"127.0.0.1","port":P<extra>}`.
fn record(port: u16, extra: &str) -> String {
    format!(r#"{{"host":"127.0.0.1","port":{port}{extra}}}"#)
}

async fn merged(record: Option<&str>, args: ConnectArgs) -> ClientInfo {
    let opts = Opts {
        record: record.map(str::to_owned),
        ..Opts::default()
    };
    let mut f = setup(opts).await;
    f.connect_ok(args).await;
    f.info()
}
