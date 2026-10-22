// Tests for `pause`, `resume`, `destroy`: INFO texts, node calls (`dht.suspend/resume/destroy`), `state` (client.md «The rest of the API»).
// Mock network; shared helpers are in `crate::support`, `in_order` is at the bottom.
use crate::support::*;
use tokio::net::TcpStream;

mod pause {
    use super::*;

    #[tokio::test]
    async fn logs_pausing_then_paused_and_suspends_the_node() {
        let mut f = setup(Opts::default()).await;
        f.start().await;
        within("pause", f.client.pause()).await.expect("pause");
        assert!(
            in_order(
                &f.lines(),
                &["[INFO] Pausing client", "[INFO] Client paused"]
            ),
            "{:?}",
            f.lines()
        );
        assert_eq!(f.named("dht.suspend"), vec![ev("dht.suspend")]);
    }

    #[tokio::test]
    async fn state_becomes_paused() {
        let mut f = setup(Opts::default()).await;
        f.start().await;
        within("pause", f.client.pause()).await.expect("pause");
        assert_eq!(f.info().state, Some("paused"));
    }
}

mod resume {
    use super::*;

    #[tokio::test]
    async fn logs_resuming_then_resumed_and_resumes_the_node_after_suspend() {
        let mut f = setup(Opts::default()).await;
        f.start().await;
        within("pause", f.client.pause()).await.expect("pause");
        within("resume", f.client.resume()).await.expect("resume");
        let want = [
            "[INFO] Pausing client",
            "[INFO] Client paused",
            "[INFO] Resuming client",
            "[INFO] Client resumed",
        ];
        assert!(in_order(&f.lines(), &want), "{:?}", f.lines());
        let calls: Vec<_> = f
            .trace()
            .into_iter()
            .filter(|e| e["ev"] != "mutableGet" && e["ev"] != "HyperDHT.new")
            .collect();
        assert_eq!(calls, vec![ev("dht.suspend"), ev("dht.resume")]);
    }

    #[tokio::test]
    async fn state_becomes_listening_again() {
        let mut f = setup(Opts::default()).await;
        f.start().await;
        within("pause", f.client.pause()).await.expect("pause");
        within("resume", f.client.resume()).await.expect("resume");
        assert_eq!(f.info().state, Some("listening"));
    }
}

mod destroy {
    use super::*;

    #[tokio::test]
    async fn logs_destroying_then_destroyed_and_destroys_the_node() {
        let mut f = setup(Opts::default()).await;
        f.start().await;
        within("destroy", f.client.destroy())
            .await
            .expect("destroy");
        let want = ["[INFO] Destroying client", "[INFO] Client destroyed"];
        assert!(in_order(&f.lines(), &want), "{:?}", f.lines());
        assert_eq!(f.named("dht.destroy"), vec![ev("dht.destroy")]);
    }

    #[tokio::test]
    async fn state_becomes_destroyed_and_the_local_port_is_closed() {
        let mut f = setup(Opts::default()).await;
        let port = f.start().await;
        within("destroy", f.client.destroy())
            .await
            .expect("destroy");
        assert_eq!(f.info().state, Some("destroyed"));
        let late = within(
            "connect after destroy",
            TcpStream::connect(("127.0.0.1", port)),
        )
        .await;
        assert!(late.is_err(), "the port no longer accepts after destroy");
    }
}

/// The `want` lines occur in `lines` in this order (other lines may appear between them).
fn in_order(lines: &[String], want: &[&str]) -> bool {
    let mut rest = lines.iter();
    want.iter().all(|w| rest.any(|l| l == w))
}
