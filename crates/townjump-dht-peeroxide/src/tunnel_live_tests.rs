// Live tests of the adapter tunnels against a JS peer (`js/peer.cjs`, hyperdht 6.34.0) on the JS testnet.
// JS: `dht.suspend()` tears down all connections (the remote peer: ECONNRESET and close within milliseconds), `connectionKeepAlive`
// of 5 s keeps an idle tunnel alive; the adapter must behave the same. Label keys 0xc1…0xc8 (0xc4 and 0xc8 are free).
use crate::dht::PeeroxideDht;
use crate::js_peer_support_tests::{JsPeer, ms_between};
use crate::server::PeeroxideServer;
use std::io;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt, ReadHalf, split};
use tokio::task::JoinHandle;
use townjump_dht::{
    ConnectOpts, Dht, DhtConnection, DhtFactory, DhtServer, Duplex, Incoming, ServerOpts,
};
use townjump_testkit::conformance::wait::{ACCEPT_WAIT, STREAM_WAIT, within};
use townjump_testkit::key_pair;

// One helper file is included in several test files: a separate copy in each would be pointless.
#[allow(clippy::duplicate_mod)]
#[path = "support_tests.rs"]
mod support;
use support::{Testnet, start_testnet};

/// JS `setTimeout` on the remote peer socket: after this long idle without incoming frames it tears the tunnel down.
const JS_TIMEOUT_MS: u64 = 7000;
/// Tunnel idle time: longer than the JS timeout and longer than one keepalive period (5 s).
const IDLE: Duration = Duration::from_secs(12);
/// Abort on suspend: JS must see ECONNRESET and close no later than this (typically ~3 ms).
const TEAR_WAIT: Duration = Duration::from_secs(3);
/// The Rust reader after suspend gets `ConnectionAborted` no later than this.
const ABORT_WAIT: Duration = Duration::from_secs(1);
/// After `resume` the DHT is still coming up: connect is retried up to this cap.
const RESUME_CONNECT_WAIT: Duration = Duration::from_secs(10);
/// After `resume` the server announces itself again (~1.5 s): dial is retried up to this cap.
const RESUME_DIAL_WAIT: Duration = Duration::from_secs(15);
const EVENT_WAIT: Duration = Duration::from_secs(5);

type Reader = JoinHandle<(Instant, io::Result<Vec<u8>>)>;

fn bootstrap_of(testnet: &Testnet) -> Vec<String> {
    let bootstrap = testnet.factory.bootstrap.clone();
    bootstrap.expect("testnet factory without bootstrap")
}

async fn rust_node(testnet: &Testnet, tag: u8) -> PeeroxideDht {
    let created = testnet.factory.create(Some(key_pair(tag)));
    let node = within(ACCEPT_WAIT, "node create", created).await;
    node.expect("node create")
}

/// A Rust server on a key by label: the node, the server (held until the end of the test) and the incoming queue.
struct RustServer {
    node: PeeroxideDht,
    _server: PeeroxideServer,
    incoming: Incoming,
}

async fn rust_server(testnet: &Testnet, tag: u8) -> RustServer {
    let node = rust_node(testnet, tag).await;
    let created = node.create_server(ServerOpts::default());
    let (server, incoming) = created.expect("create_server");
    let listen = server.listen(key_pair(tag));
    within(ACCEPT_WAIT, "listen", listen).await.expect("listen");
    RustServer {
        node,
        _server: server,
        incoming,
    }
}

/// `connect` with retries: right after `resume` the node is still coming up and an attempt error is not a failure of the test.
async fn connect_retry(node: &PeeroxideDht, pk: [u8; 32], total: Duration) -> DhtConnection {
    let deadline = Instant::now() + total;
    let mut last = "no attempts made".to_string();
    while Instant::now() < deadline {
        let left = deadline.saturating_duration_since(Instant::now());
        match tokio::time::timeout(left, node.connect(pk, ConnectOpts::default())).await {
            Ok(Ok(conn)) => return conn,
            Ok(Err(e)) => last = e.to_string(),
            Err(_) => break,
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    panic!(
        "connect to JS failed within {} s; last error: {last}",
        total.as_secs()
    )
}

/// Writes `text`, JS prints it as DATA, the echo returns on the same stream.
async fn echo_roundtrip(stream: &mut Box<dyn Duplex>, js: &JsPeer, text: &str) {
    stream.write_all(text.as_bytes()).await.expect("write");
    stream.flush().await.expect("flush");
    let seen = js.wait_event("DATA", STREAM_WAIT).await;
    let seen = seen.unwrap_or_else(|| panic!("JS did not print DATA {text}: {}", js.dump()));
    assert_eq!(seen.data, text, "JS received different bytes");
    let mut echo = vec![0u8; text.len()];
    within(STREAM_WAIT, "echo from JS", stream.read_exact(&mut echo))
        .await
        .expect("read of the echo");
    assert_eq!(echo, text.as_bytes(), "echo differs from what was written");
}

/// The reader stays blocked in a single `read` (as townjump-pipe does); the result and the return time are reported at the end.
fn spawn_reader(mut rd: ReadHalf<Box<dyn Duplex>>) -> Reader {
    tokio::spawn(async move {
        let mut buf = [0u8; 64];
        let read = rd.read(&mut buf).await;
        (Instant::now(), read.map(|n| buf[..n].to_vec()))
    })
}

/// The JS side saw `ERROR ECONNRESET` and `CLOSE`, both no later than `TEAR_WAIT` after `t0` (the moment suspend was called).
async fn assert_js_torn(js: &JsPeer, t0: Instant, what: &str) {
    let dump = || js.dump();
    let error = js.wait_event("ERROR", TEAR_WAIT).await;
    let error = error.unwrap_or_else(|| panic!("{what}: JS got no ERROR within 3 s: {}", dump()));
    let close = js.wait_event("CLOSE", TEAR_WAIT).await;
    let close = close.unwrap_or_else(|| panic!("{what}: JS got no CLOSE within 3 s: {}", dump()));
    let (to_error, to_close) = (ms_between(t0, error.at), ms_between(t0, close.at));
    eprintln!(
        "{what}: JS ERROR {} after {to_error:.2} ms, CLOSE after {to_close:.2} ms from the suspend() call",
        error.data
    );
    assert_eq!(error.data, "ECONNRESET", "{what}: JS error code");
    assert!(
        to_close <= TEAR_WAIT.as_secs_f64() * 1000.0,
        "{what}: CLOSE after {to_close} ms"
    );
    assert!(
        to_error >= 0.0,
        "{what}: ERROR came {to_error} ms before the suspend call"
    );
}

/// The Rust reader returned `ConnectionAborted` no later than `ABORT_WAIT` after `t0`.
async fn assert_reader_aborted(reader: Reader, t0: Instant, what: &str) {
    let done = within(TEAR_WAIT, "reader after suspend", reader).await;
    let (at, read) = done.expect("reader task");
    let took = ms_between(t0, at);
    eprintln!("{what}: the Rust reader got {read:?} {took:.2} ms after the suspend() call");
    let error = read.expect_err("read after suspend must end with an error, not EOF");
    assert_eq!(
        error.kind(),
        io::ErrorKind::ConnectionAborted,
        "{what}: {error}"
    );
    assert!(
        took < ABORT_WAIT.as_secs_f64() * 1000.0,
        "{what}: the reader waited {took} ms"
    );
}

/// Idle: JS prints no `ERROR`/`CLOSE`, the Rust reader stays blocked in `read` the whole time.
async fn assert_idle_quiet(js: &JsPeer, reader: &Reader, what: &str) {
    tokio::time::sleep(IDLE).await;
    let torn = js.seen("ERROR") || js.seen("CLOSE");
    assert!(
        !torn,
        "{what}: JS tore down an idle tunnel by itself: {}",
        js.dump()
    );
    assert!(
        !reader.is_finished(),
        "{what}: the Rust reader returned during the idle time"
    );
}

/// The reader returned exactly `want` (the echo or data from JS).
async fn assert_reader_got(reader: Reader, want: &str, what: &str) {
    let done = within(STREAM_WAIT, what, reader).await;
    let (_, read) = done.expect("reader task");
    assert_eq!(read.expect("read"), want.as_bytes(), "{what}");
}

mod client_side {
    use super::*;

    mod suspend {
        use super::*;

        // JS (testvectors/probes-js/fork-suspend-probe-suspend.cjs): dht.suspend() → the remote peer gets ECONNRESET and close within ~3 ms.
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        #[ignore = "needs the JS testnet: node and .tmp/npm-holesail"]
        async fn rust_client_tunnel_is_torn_by_suspend() {
            let testnet = start_testnet();
            let js_pk = key_pair(0xc1).public_key;
            let js = JsPeer::serve(&bootstrap_of(&testnet), [0xc1; 32], None).await;
            let node = rust_node(&testnet, 0xc2).await;
            let mut conn = connect_retry(&node, js_pk, ACCEPT_WAIT).await;
            assert_eq!(conn.remote_public_key, js_pk);
            js.wait_event("CONNECTION", EVENT_WAIT)
                .await
                .expect("CONNECTION");
            echo_roundtrip(&mut conn.stream, &js, "ping").await;
            let (rd, _wr) = split(conn.stream);
            let reader = spawn_reader(rd);
            let t0 = Instant::now();
            within(ACCEPT_WAIT, "suspend", node.suspend())
                .await
                .expect("suspend");
            assert_js_torn(&js, t0, "Rust client").await;
            assert_reader_aborted(reader, t0, "Rust client").await;
            within(ACCEPT_WAIT, "resume", node.resume())
                .await
                .expect("resume");
            let mut again = connect_retry(&node, js_pk, RESUME_CONNECT_WAIT).await;
            js.wait_event("CONNECTION", EVENT_WAIT)
                .await
                .expect("CONNECTION after resume");
            echo_roundtrip(&mut again.stream, &js, "again").await;
        }
    }

    mod keepalive {
        use super::*;

        // JS: socket.setTimeout(7000) on the server side; only an empty Rust frame every 5 s keeps the tunnel alive.
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        #[ignore = "needs the JS testnet: node and .tmp/npm-holesail"]
        async fn idle_client_tunnel_survives_the_js_timeout() {
            let testnet = start_testnet();
            let js_pk = key_pair(0xc5).public_key;
            let boot = bootstrap_of(&testnet);
            let js = JsPeer::serve(&boot, [0xc5; 32], Some(JS_TIMEOUT_MS)).await;
            let node = rust_node(&testnet, 0xc6).await;
            let conn = connect_retry(&node, js_pk, ACCEPT_WAIT).await;
            js.wait_event("CONNECTION", EVENT_WAIT)
                .await
                .expect("CONNECTION");
            let (rd, mut wr) = split(conn.stream);
            let reader = spawn_reader(rd);
            assert_idle_quiet(&js, &reader, "client idle").await;
            wr.write_all(b"late").await.expect("write late");
            wr.flush().await.expect("flush late");
            let data = js.wait_event("DATA", STREAM_WAIT).await.expect("DATA late");
            assert_eq!(data.data, "late", "JS received different bytes");
            assert_reader_got(reader, "late", "echo after idle").await;
        }
    }
}

mod server_side {
    use super::*;

    /// JS `dial` to the Rust server key: `OPEN` on the JS side and `accept` on the Rust side.
    async fn dial_and_accept(
        boot: &[String],
        srv: &mut RustServer,
        tag: u8,
        ms: Option<u64>,
    ) -> (JsPeer, DhtConnection) {
        let js = JsPeer::dial(boot, key_pair(tag).public_key, ms);
        let opened = js.wait_event("OPEN", ACCEPT_WAIT);
        let accepted = within(ACCEPT_WAIT, "accept", srv.incoming.accept());
        let (opened, accepted) = tokio::join!(opened, accepted);
        assert!(opened.is_some(), "JS dial did not open: {}", js.dump());
        (js, accepted.expect("server closed, no incoming connection"))
    }

    /// A new JS `dial` after `resume`: the announce does not resume immediately, so failed attempts are retried.
    async fn redial(boot: &[String], srv: &mut RustServer, tag: u8) -> (JsPeer, DhtConnection) {
        let deadline = Instant::now() + RESUME_DIAL_WAIT;
        loop {
            let js = JsPeer::dial(boot, key_pair(tag).public_key, None);
            let left = deadline.saturating_duration_since(Instant::now());
            let event = js.wait_any(&["OPEN", "ERROR"], left.min(EVENT_WAIT)).await;
            if event.is_some_and(|e| e.name == "OPEN") {
                let accepted = within(ACCEPT_WAIT, "accept after resume", srv.incoming.accept());
                return (js, accepted.await.expect("server closed"));
            }
            assert!(
                Instant::now() < deadline,
                "dial after resume failed within 15 s: {}",
                js.dump()
            );
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
    }

    mod suspend {
        use super::*;

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        #[ignore = "needs the JS testnet: node and .tmp/npm-holesail"]
        async fn rust_server_tunnel_is_torn_by_suspend() {
            let testnet = start_testnet();
            let boot = bootstrap_of(&testnet);
            let mut srv = rust_server(&testnet, 0xc3).await;
            let (js, mut conn) = dial_and_accept(&boot, &mut srv, 0xc3, None).await;
            echo_roundtrip(&mut conn.stream, &js, "ping").await;
            let (rd, _wr) = split(conn.stream);
            let reader = spawn_reader(rd);
            let t0 = Instant::now();
            within(ACCEPT_WAIT, "suspend", srv.node.suspend())
                .await
                .expect("suspend");
            assert_js_torn(&js, t0, "Rust server").await;
            assert_reader_aborted(reader, t0, "Rust server").await;
            within(ACCEPT_WAIT, "resume", srv.node.resume())
                .await
                .expect("resume");
            let (js2, mut again) = redial(&boot, &mut srv, 0xc3).await;
            echo_roundtrip(&mut again.stream, &js2, "again").await;
        }
    }

    mod keepalive {
        use super::*;

        // JS: socket.setTimeout(7000) on the dial side; the accepted Rust stream is silent for 12 s, only keepalive sends frames.
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        #[ignore = "needs the JS testnet: node and .tmp/npm-holesail"]
        async fn idle_server_tunnel_survives_the_js_timeout() {
            let testnet = start_testnet();
            let boot = bootstrap_of(&testnet);
            let mut srv = rust_server(&testnet, 0xc7).await;
            let (mut js, conn) = dial_and_accept(&boot, &mut srv, 0xc7, Some(JS_TIMEOUT_MS)).await;
            let (rd, _wr) = split(conn.stream);
            let reader = spawn_reader(rd);
            assert_idle_quiet(&js, &reader, "server idle").await;
            js.send_line("send ping");
            assert_reader_got(reader, "ping", "data from JS after idle").await;
        }
    }
}
