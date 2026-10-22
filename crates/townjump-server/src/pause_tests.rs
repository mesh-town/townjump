// Tests for `pause`, `resume`, `destroy`: log lines, DHT events, state, accepting connections.
// Source: server.md «pause/resume/destroy», JS holesail-server/index.js (pause, resume, destroy).
use super::fixture::*;
use std::time::Duration;
use tokio::time::sleep;
use townjump_dht::DhtError;
use townjump_testkit::mock::events::{destroy_event, resume_event, suspend_event};

mod pause {
    use super::*;

    #[tokio::test]
    async fn logs_around_dht_suspend() {
        let mut fx = running().await;
        let (log, trace) = (fx.log().len(), fx.trace.events().len());
        fx.server.pause().await.unwrap();
        assert_eq!(
            fx.log()[log..],
            [info("Pausing server"), info("Server paused")]
        );
        assert_eq!(fx.trace.events()[trace..], [suspend_event()]);
    }

    #[tokio::test]
    async fn state_becomes_paused() {
        let mut fx = running().await;
        fx.server.pause().await.unwrap();
        assert_eq!(fx.server.info().state, Some("paused"));
    }

    #[tokio::test]
    async fn new_connections_are_refused_while_paused() {
        let mut fx = running().await;
        fx.server.pause().await.unwrap();
        let refused = fx.connect_as(server_pair()).await;
        assert!(
            matches!(refused, Err(DhtError::ConnectionFailed(_))),
            "{refused:?}"
        );
    }
}

mod resume {
    use super::*;

    #[tokio::test]
    async fn logs_around_dht_resume() {
        let mut fx = paused().await;
        let (log, trace) = (fx.log().len(), fx.trace.events().len());
        fx.server.resume().await.unwrap();
        assert_eq!(
            fx.log()[log..],
            [info("Resuming server"), info("Server resumed")]
        );
        assert_eq!(fx.trace.events()[trace..], [resume_event()]);
    }

    #[tokio::test]
    async fn state_becomes_listening_again() {
        let mut fx = paused().await;
        fx.server.resume().await.unwrap();
        assert_eq!(fx.server.info().state, Some("listening"));
    }

    #[tokio::test]
    async fn connections_are_accepted_again() {
        let mut fx = paused().await;
        fx.server.resume().await.unwrap();
        fx.connect_as(server_pair())
            .await
            .expect("rejection after resume");
    }
}

mod destroy {
    use super::*;

    #[tokio::test]
    async fn logs_the_three_info_lines_and_destroys_the_dht_last() {
        let mut fx = running().await;
        let (log, trace) = (fx.log().len(), fx.trace.events().len());
        fx.server.destroy().await.unwrap();
        let lines = [
            "Destroying server",
            "Cleared DHT refresh interval",
            "Server destroyed",
        ];
        assert_eq!(fx.log()[log..], lines.map(info));
        assert_eq!(fx.trace.events()[trace..], [destroy_event()]);
    }

    #[tokio::test]
    async fn incoming_is_closed_so_nobody_can_connect() {
        let mut fx = running().await;
        fx.server.destroy().await.unwrap();
        let refused = fx.connect_as(server_pair()).await;
        assert!(
            matches!(refused, Err(DhtError::PeerNotFound)),
            "{refused:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn stops_the_refresh_timer() {
        let mut fx = running().await;
        fx.server.destroy().await.unwrap();
        sleep(Duration::from_secs(3 * 3600)).await;
        assert_eq!(fx.factory.put_calls(), 1, "put after destroy");
        assert!(fx.fatal.try_recv().is_err(), "Fatal after destroy");
    }
}

/// Running server, `listen` finished.
async fn running() -> Fixture {
    let fx = Fixture::started(Setup {
        port: free_port().await,
        ..Setup::default()
    })
    .await;
    fx.wait_listening().await;
    fx
}

async fn paused() -> Fixture {
    let mut fx = running().await;
    fx.server.pause().await.unwrap();
    fx
}

fn info(msg: &str) -> (String, String) {
    ("INFO".to_string(), msg.to_string())
}
