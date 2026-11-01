// Tests of the LAN responder lifecycle (registration, close, suspend, destroy) and of the probe trigger (virtual time).
// A child of lan_race_tests: shares its helpers.
use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lan_discovery_errors_do_not_fail_the_dht_path() {
    let _lan = LAN.lock().await;
    let net = support::start_testnet();
    // Port 0: sends fail. A held port: the responder cannot start. Neither may touch the DHT path.
    let held = UdpSocket::bind("0.0.0.0:0").expect("held");
    let busy = held.local_addr().expect("addr").port();
    let server = node(&net.factory, setup(busy, ProbeStart::Now)).await;
    let (srv, mut incoming) = server.create_server(ServerOpts::default()).expect("server");
    let key = key_pair(SEED);
    within(WAIT, "listen", srv.listen(key.clone()))
        .await
        .expect("listen");
    let client = node(&net.factory, setup(0, ProbeStart::Now)).await;
    let (conn, _) = tokio::join!(
        within(
            WAIT,
            "connect",
            client.connect(key.public_key, Default::default())
        ),
        accept_one(&mut incoming, "accept")
    );
    assert!(conn.is_ok(), "{:?}", conn.err());
    let _ = tokio::join!(client.destroy(), server.destroy());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lan_answers_only_once_the_server_accepts_handshakes() {
    let _lan = LAN.lock().await;
    let port = group_port();
    let dht = node(&closed_factory(), setup(port, ProbeStart::Now)).await;
    let (server, _incoming) = dht.create_server(ServerOpts::default()).expect("server");
    let key = key_pair(SEED);
    let listen = tokio::spawn({
        let key = key.clone();
        async move { server.listen(key).await }
    });
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(
        !listen.is_finished(),
        "listen must still wait for the bootstrap"
    );
    assert_eq!(
        ask(port, &key).await,
        None,
        "answered before the server accepts handshakes"
    );
    within(WAIT, "listen", listen)
        .await
        .expect("join")
        .expect("listen");
    let local = dht.local_port().await.expect("port");
    assert_eq!(ask_some(port, &key).await.map(|a| a.port()), Some(local));
    let _ = dht.destroy().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn close_stops_lan_answers() {
    let _lan = LAN.lock().await;
    let port = group_port();
    let (dht, server, _incoming, key) = serving(port, &closed_factory()).await;
    assert!(ask_some(port, &key).await.is_some());
    server.close().await.expect("close");
    assert_eq!(ask(port, &key).await, None);
    let _ = dht.destroy().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn suspended_node_does_not_answer_on_the_lan() {
    let _lan = LAN.lock().await;
    let port = group_port();
    let (dht, _server, _incoming, key) = serving(port, &closed_factory()).await;
    dht.suspend().await.expect("suspend");
    assert_eq!(ask(port, &key).await, None, "a suspended node answered");
    dht.resume().await.expect("resume");
    assert!(
        ask_some(port, &key).await.is_some(),
        "no answer after resume"
    );
    let _ = dht.destroy().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn destroyed_node_stops_answering() {
    let _lan = LAN.lock().await;
    let port = group_port();
    let (dht, _server, _incoming, key) = serving(port, &closed_factory()).await;
    assert!(ask_some(port, &key).await.is_some());
    dht.destroy().await.expect("destroy");
    assert_eq!(ask(port, &key).await, None);
}

mod trigger {
    use super::*;
    use crate::lan_race::PROBE_START;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const EMPTY: Table = Table {
        bootstrapped: false,
        size: 0,
    };

    /// Virtual time from the call to the moment `wait_start` resolves; `fail_at` - when the DHT path fails.
    async fn started_after(
        mode: ProbeStart,
        table: impl Fn(usize) -> Table,
        fail_at: Option<Duration>,
    ) -> Duration {
        let (failed, calls) = (Notify::new(), AtomicUsize::new(0));
        let state = || {
            let t = table(calls.fetch_add(1, Ordering::SeqCst));
            async move { t }
        };
        let begun = tokio::time::Instant::now();
        let start = async {
            wait_start(mode, state, &failed).await;
            begun.elapsed()
        };
        let fail = async {
            if let Some(d) = fail_at {
                tokio::time::sleep(d).await;
                failed.notify_one();
            }
        };
        tokio::join!(start, fail).0
    }

    #[tokio::test(start_paused = true)]
    async fn the_probe_starts_at_once_by_default() {
        assert_eq!(PROBE_START, ProbeStart::Now);
        let full = Table {
            bootstrapped: true,
            size: 5,
        };
        let took = started_after(PROBE_START, |_| full, None).await;
        assert!(took < Duration::from_millis(50), "took {took:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn probe_starts_at_once() {
        assert_eq!(
            started_after(ProbeStart::Now, |_| EMPTY, None).await,
            Duration::ZERO
        );
    }

    #[tokio::test(start_paused = true)]
    async fn probe_starts_at_once_when_the_table_is_known_empty() {
        let done = Table {
            bootstrapped: true,
            size: 0,
        };
        assert_eq!(
            started_after(ProbeStart::Quiet, |_| done, None).await,
            Duration::ZERO
        );
    }

    #[tokio::test(start_paused = true)]
    async fn probe_waits_for_an_empty_table_after_the_silence() {
        let took = started_after(ProbeStart::Quiet, |_| EMPTY, None).await;
        assert!(
            took >= SILENCE && took < SILENCE + Duration::from_millis(250),
            "took {took:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn probe_starts_after_the_dht_fails() {
        let at = Duration::from_secs(2);
        let took = started_after(ProbeStart::AfterFailure, |_| EMPTY, Some(at)).await;
        assert_eq!(took, at);
    }

    #[tokio::test(start_paused = true)]
    async fn probe_is_skipped_while_the_table_fills() {
        // The table fills at the third poll; no probe at the silence, only at the DHT failure.
        let filling = |n| Table {
            bootstrapped: false,
            size: if n < 2 { 0 } else { 5 },
        };
        let at = Duration::from_secs(3);
        let took = started_after(ProbeStart::Quiet, filling, Some(at)).await;
        assert_eq!(took, at, "the probe started before the DHT failed");
    }
}
