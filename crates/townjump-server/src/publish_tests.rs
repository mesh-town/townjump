// Tests of `record::publish`: a put that no DHT node answered must not stop the server (start and refresh).
// JS differs on purpose: there the unhandled rejection kills the process; the port keeps LAN-only servers alive.
use super::fixture::*;
use super::*;
use std::time::Duration;
use tokio::time::{sleep, timeout};
use townjump_dht::NO_NODES_RESPONDED;
use townjump_testkit::mock::MockConfig;

const WARNING: &str =
    "DHT record not published: no DHT node responded (will retry on the next refresh)";

fn minutes(n: u64) -> Duration {
    Duration::from_secs(n * 60)
}

fn no_responders_from(call: usize) -> Setup {
    Setup {
        mock: MockConfig::default().failing_put_with(call, NO_NODES_RESPONDED),
        ..Setup::default()
    }
}

#[tokio::test]
async fn first_put_without_responders_does_not_fail_start() {
    let fx = Fixture::started(no_responders_from(1)).await;
    assert!(fx.err.text().contains(WARNING), "{:?}", fx.err.text());
    assert_eq!(fx.factory.put_calls(), 1);
}

#[tokio::test(start_paused = true)]
async fn refresh_without_responders_is_not_fatal_and_repeats() {
    let mut fx = Fixture::started(no_responders_from(2)).await;
    sleep(minutes(51)).await;
    assert!(
        timeout(SILENCE, fx.fatal.recv()).await.is_err(),
        "no-responders refresh went to Fatal"
    );
    assert!(fx.err.text().contains(WARNING), "{:?}", fx.err.text());
    assert_eq!(fx.factory.put_calls(), 2);
    sleep(minutes(50)).await;
    assert_eq!(fx.factory.put_calls(), 3, "the refresh loop stopped");
}

#[tokio::test]
async fn direct_put_without_responders_is_returned_and_not_fatal() {
    let mut fx = Fixture::started(no_responders_from(2)).await;
    let error = fx.server.put("{}").await.expect_err("put hid the error");
    assert!(
        matches!(&error, ServerError::Dht(e) if e.is_no_nodes_responded()),
        "{error:?}"
    );
    assert!(
        timeout(SILENCE, fx.fatal.recv()).await.is_err(),
        "call error went to Fatal"
    );
}
