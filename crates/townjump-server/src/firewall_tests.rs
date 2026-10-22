// Server firewall tests: tests 1-4 of firewall.md (the client is a `Dht` of the same factory, calls `connect`), a callback probe.
// Test 5 (the callback throws an exception) is the DHT layer, the server crate does not build it.
use super::fixture::*;
use serde_json::json;
use tokio::time::sleep;
use townjump_dht::DhtError;
use townjump_testkit::key_pair;

mod private_mode {
    use super::*;

    mod client_with_the_server_key {
        use super::*;

        #[tokio::test]
        async fn is_accepted_and_logged() {
            let fx = quiet_server(true).await;
            fx.connect_as(server_pair())
                .await
                .expect("own key rejected");
            fx.wait_log("DEBUG", &incoming_line(&server_pair().public_key))
                .await;
        }
    }

    mod client_with_another_key {
        use super::*;

        #[tokio::test]
        async fn is_refused_with_connection_failed() {
            let fx = quiet_server(true).await;
            let error = fx
                .connect_as(key_pair(3))
                .await
                .expect_err("foreign key accepted");
            assert!(matches!(error, DhtError::ConnectionFailed(_)), "{error:?}");
            assert_eq!(
                error.to_string(),
                "PEER_CONNECTION_FAILED: Could not connect to peer"
            );
        }

        #[tokio::test]
        async fn leaves_no_incoming_line() {
            let fx = quiet_server(true).await;
            let _refused = fx.connect_as(key_pair(3)).await;
            sleep(SILENCE).await;
            assert!(!has_incoming(&fx), "{:?}", fx.log());
        }

        #[tokio::test]
        async fn does_not_stop_the_server_serving_its_own_key() {
            let fx = quiet_server(true).await;
            let _refused = fx.connect_as(key_pair(3)).await;
            fx.connect_as(server_pair())
                .await
                .expect("after a rejection the own key was not accepted");
        }
    }

    mod client_key_one_bit_off {
        use super::*;

        #[tokio::test]
        async fn is_refused() {
            let fx = quiet_server(true).await;
            let mut near = server_pair();
            near.public_key[0] ^= 1;
            let refused = fx.connect_as(near).await;
            assert!(refused.is_err(), "key with one differing bit accepted");
        }
    }
}

mod public_mode {
    use super::*;

    #[tokio::test]
    async fn client_with_any_key_is_accepted() {
        let fx = quiet_server(false).await;
        fx.connect_as(key_pair(3))
            .await
            .expect("public server refused");
        fx.wait_log("DEBUG", &incoming_line(&key_pair(3).public_key))
            .await;
    }

    #[tokio::test]
    async fn client_with_the_server_key_is_accepted_too() {
        let fx = quiet_server(false).await;
        fx.connect_as(server_pair())
            .await
            .expect("public server refused");
    }
}

// How the server builds the callback: `firewall: "function"` only in private mode; `listen` tries it with two keys.
mod callback {
    use super::*;

    #[tokio::test]
    async fn private_mode_probes_own_key_false_and_other_key_true() {
        let fx = quiet_server(true).await;
        let probes = fx.trace.named("firewall.probe");
        assert_eq!(probes.len(), 1, "{probes:?}");
        assert_eq!(
            (&probes[0]["ownKey"], &probes[0]["otherKey"]),
            (&json!(false), &json!(true))
        );
    }

    #[tokio::test]
    async fn private_mode_creates_the_server_with_a_function_firewall() {
        let fx = quiet_server(true).await;
        assert_eq!(create_server_event(&fx)["firewall"], json!("function"));
    }

    #[tokio::test]
    async fn public_mode_creates_the_server_with_a_boolean_firewall_and_no_probe() {
        let fx = quiet_server(false).await;
        assert_eq!(create_server_event(&fx)["firewall"], json!("boolean"));
        assert!(fx.trace.named("firewall.probe").is_empty());
    }

    #[tokio::test]
    async fn both_modes_request_a_reusable_socket() {
        for secure in [true, false] {
            let fx = quiet_server(secure).await;
            assert_eq!(
                create_server_event(&fx)["reusableSocket"],
                json!(true),
                "secure = {secure}"
            );
        }
    }
}

/// Running server; the local port is free, nobody connects to it.
async fn quiet_server(secure: bool) -> Fixture {
    Fixture::started(Setup {
        secure,
        port: free_port().await,
        ..Setup::default()
    })
    .await
}

fn incoming_line(remote: &[u8; 32]) -> String {
    format!("Incoming connection received from {}", z32(remote))
}

fn has_incoming(fx: &Fixture) -> bool {
    let is_incoming = |msg: &String| msg.starts_with("Incoming connection received from");
    fx.messages("DEBUG").iter().any(is_incoming)
}

fn create_server_event(fx: &Fixture) -> serde_json::Value {
    fx.trace.named("createServer").remove(0)
}
