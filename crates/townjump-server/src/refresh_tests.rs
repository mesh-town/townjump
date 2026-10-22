// Tests for the record refresh every 50 minutes and `Fatal` on `listen` and refresh errors.
// Source: server.md «start» step 7 and «Fatal»; JS holesail-server/index.js (setInterval 50 * 60 * 1000).
use super::fixture::*;
use super::*;
use std::time::Duration;
use tokio::time::{sleep, timeout};
use townjump_dht::{Dht, DhtError};
use townjump_testkit::key_pair;
use townjump_testkit::mock::MockConfig;

const RECORD: &str = r#"{"host":"127.0.0.1","port":3000}"#;

fn minutes(n: u64) -> Duration {
    Duration::from_secs(n * 60)
}

mod refresh {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn does_not_fire_before_50_minutes() {
        let fx = Fixture::started(Setup::default()).await;
        sleep(minutes(49)).await;
        assert_eq!(fx.factory.put_calls(), 1, "put before the interval expires");
    }

    #[tokio::test(start_paused = true)]
    async fn puts_the_same_value_with_the_same_seq_after_50_minutes() {
        let fx = Fixture::started(Setup::default()).await;
        sleep(minutes(51)).await;
        assert_eq!(fx.factory.put_calls(), 2);
        let last = fx.trace.named("mutablePut").pop().expect("no mutablePut");
        assert_eq!(
            (last["value"].as_str(), last["seq"].as_u64()),
            (Some(RECORD), Some(0))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn logs_the_refresh() {
        let fx = Fixture::started(Setup::default()).await;
        sleep(minutes(51)).await;
        assert!(
            fx.has("DEBUG", &format!("Refreshing DHT record: {RECORD}")),
            "{:?}",
            fx.log()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn repeats_every_50_minutes() {
        let fx = Fixture::started(Setup::default()).await;
        sleep(minutes(151)).await;
        assert_eq!(
            fx.factory.put_calls(),
            4,
            "start and three refreshes in 151 min"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn goes_above_a_newer_foreign_record() {
        let fx = Fixture::started(Setup::default()).await;
        let node = fx.client(key_pair(9)).await;
        node.mutable_put(&server_pair(), b"foreign", Some(7))
            .await
            .unwrap();
        sleep(minutes(51)).await;
        let last = fx.trace.named("mutablePut").pop().expect("no mutablePut");
        assert_eq!(last["seq"].as_u64(), Some(8));
    }
}

// errors of background tasks go to `Fatal`, errors of a call go to the caller.
mod fatal {
    use super::*;

    #[tokio::test]
    async fn listen_error_reaches_fatal_while_start_succeeds() {
        let mut fx = setup(with(MockConfig::default().failing_listen("listen failed"))).await;
        fx.start().await;
        let message = next_fatal(&mut fx).await;
        assert!(message.contains("listen failed"), "{message}");
    }

    #[tokio::test]
    async fn first_put_error_is_returned_from_start() {
        let mut fx = setup(with(MockConfig::default().failing_put_from(1))).await;
        let args = fx.args.clone();
        let error = fx
            .server
            .start(args)
            .await
            .expect_err("start hid the put error");
        assert!(
            matches!(error, ServerError::Dht(DhtError::Backend(ref t)) if t == "put failed"),
            "{error:?}"
        );
    }

    #[tokio::test]
    async fn direct_put_error_is_returned_and_not_fatal() {
        let mut fx = Fixture::started(with(MockConfig::default().failing_put_from(2))).await;
        let error = fx.server.put("{}").await.expect_err("put hid the error");
        assert!(
            matches!(error, ServerError::Dht(DhtError::Backend(_))),
            "{error:?}"
        );
        assert!(
            timeout(SILENCE, fx.fatal.recv()).await.is_err(),
            "call error went to Fatal"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn refresh_error_reaches_fatal() {
        let mut fx = Fixture::started(with(MockConfig::default().failing_put_from(2))).await;
        sleep(minutes(51)).await;
        let message = next_fatal(&mut fx).await;
        assert!(message.contains("put failed"), "{message}");
    }
}

fn with(mock: MockConfig) -> Setup {
    Setup {
        mock,
        ..Setup::default()
    }
}

async fn next_fatal(fx: &mut Fixture) -> String {
    let message = timeout(WAIT, fx.fatal.recv()).await;
    message
        .expect("Fatal did not fire")
        .expect("Fatal channel closed")
}
