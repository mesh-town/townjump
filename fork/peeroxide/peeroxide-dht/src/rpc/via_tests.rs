// JS: dht-rpc/index.js request({ socket }), dht-rpc/lib/io.js
//! Tests for `DhtHandle::request_via` and `forward_inbound_via` on the DHT actor: a request and its
//! retransmissions go out from the external socket E, a request accepted via E gets a response from E, a session
//! cancels retransmissions. E is held by a task: it reads `E.recv_start()` and calls `forward_inbound_via`
//! (as the `Reuse` forwarder does). R is an ordinary `tokio::net::UdpSocket`: the sender address
//! shows which socket the datagram left from. Model: `rpc_session_tests.rs` (its helpers are private).

use std::net::SocketAddr;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use libudx::{UdxRuntime, UdxSocket};
use tokio::net::UdpSocket;
use tokio::sync::mpsc::{self, UnboundedReceiver};

use super::command::DhtCommand;
use super::{DhtConfig, DhtError, DhtHandle, ResponseData, UserRequestParams, spawn};
use crate::io::WireCounters;
use crate::messages::{self, Ipv4Peer, Message};
use crate::routing_table::RoutingTable;

/// User command: the node handles PING itself, `subscribe_requests` does not see it.
const COMMAND: u64 = 77;
const REQUEST_TID: u16 = 41;
/// How long we wait for something that must happen.
const WAIT: Duration = Duration::from_secs(5);
/// How long we wait to be sure no datagram will come.
const QUIET: Duration = Duration::from_millis(400);
/// Window after a session drop: ordinary request retransmissions go once per 1000 ms, the next one would arrive inside it.
const RETRY_WINDOW: Duration = Duration::from_millis(2500);

mod channel {
    use super::*;

    #[tokio::test]
    async fn request_via_sends_session_and_via() {
        // given: a session handle and external socket E; no actor, commands are read from the channel
        let (h, mut rx) = channel_handle();
        let rt = UdxRuntime::new().expect("runtime");
        let e = rt.create_socket().await.expect("socket");
        let s = h.session();
        // when: a request via E is polled and dropped
        poll_and_drop(s.dht().request_via(params(), "127.0.0.1", 9, e.clone())).await;
        // then: the channel has a Request with the session id, socket E and the request address
        match rx.try_recv() {
            Ok(DhtCommand::Request {
                params,
                host,
                port,
                session,
                via,
                ..
            }) => {
                assert_eq!(session, Some(s.id()), "request goes in the handle session");
                assert!(
                    via.is_some_and(|v| v.ptr_eq(&e)),
                    "via is the same socket E"
                );
                assert_eq!((host.as_str(), port), ("127.0.0.1", 9));
                assert_eq!(params.command, COMMAND);
            }
            _ => panic!("expected Request"),
        }
        // then: no DestroySession while the session is alive
        assert!(rx.try_recv().is_err(), "session alive: no destroy sent");
    }

    #[tokio::test]
    async fn unscoped_request_via_runs_in_an_implicit_session_destroyed_with_the_future() {
        // given: a handle without a session and external socket E
        let (h, mut rx) = channel_handle();
        let rt = UdxRuntime::new().expect("runtime");
        let e = rt.create_socket().await.expect("socket");
        // when: a request via E is polled and dropped
        poll_and_drop(h.request_via(params(), "127.0.0.1", 9, e.clone())).await;
        // then: Request with implicit session x and socket E, then DestroySession x
        let used = match rx.try_recv() {
            Ok(DhtCommand::Request { session, via, .. }) => {
                assert!(
                    via.is_some_and(|v| v.ptr_eq(&e)),
                    "via is the same socket E"
                );
                session.expect("no request goes out without a session")
            }
            _ => panic!("expected Request"),
        };
        match rx.try_recv() {
            Ok(DhtCommand::DestroySession { id }) => assert_eq!(id, used),
            _ => panic!("expected DestroySession of the implicit session"),
        }
    }
}

mod live_nodes {
    use super::*;

    #[tokio::test]
    async fn request_via_reaches_b_from_e() {
        // given: nodes A and B, E at A; B is subscribed to requests
        let lab = setup().await;
        let b = live_node(&lab.rt).await;
        let b_port = b.local_port().await.expect("port of B");
        let mut b_requests = b.subscribe_requests().await.expect("subscription of B");
        // when: A sends B a user command via E
        let call = lab.when_requests_via_e(&lab.a, b_port);
        // then: B sees the request from the address of E, not from the socket of A
        let mut req = within(b_requests.recv()).await.expect("request at B");
        assert_eq!(req.from, lab.e_peer(), "B sees the address of E");
        assert_eq!(req.command, COMMAND);
        // then: the response of B reaches A via E: the request completed successfully
        req.reply(Some(b"pong".to_vec()));
        let resp = within(call).await.expect("task").expect("response");
        assert_eq!(resp.error, 0);
        assert_eq!(resp.value, Some(b"pong".to_vec()));
        assert_eq!(resp.from, peer(b_port));
    }

    #[tokio::test]
    async fn user_request_via_e_replied_from_e() {
        // given: a node, E at it, R is an external peer
        let lab = setup().await;
        let mut requests = lab.a.subscribe_requests().await.expect("subscription");
        let r = Remote::start().await;
        // when: a request from R is accepted via E
        r.send_request(lab.e_addr, REQUEST_TID).await;
        let mut req = within(requests.recv()).await.expect("request at the node");
        // then: a request from R, arrived on socket E
        assert_eq!(req.from, peer(r.addr.port()));
        assert_eq!(req.tid, REQUEST_TID);
        assert!(req.arrived_on.ptr_eq(&lab.e), "arrived_on is socket E");
        assert_eq!(req.arrived_on.local_addr().await.expect("addr"), lab.e_addr);
        // when: the user replies
        req.reply(Some(b"ok".to_vec()));
        // then: R gets a response from the address of E, on the same tid
        let (data, src) = r.recv_within(WAIT).await.expect("response at R");
        assert_eq!(src, lab.e_addr, "response did not go out from E");
        let res = decode_response(&data);
        assert_eq!(res.tid, REQUEST_TID);
        assert_eq!((res.error, res.value), (0, Some(b"ok".to_vec())));
    }

    #[tokio::test]
    async fn dropping_session_stops_via_retries() {
        // given: a request via E in a session to a silent address; the first send and the first retransmission are from E
        let lab = setup().await;
        let silent = SilentPeer::start().await;
        let session = lab.a.session();
        let _call = lab.when_requests_via_e(session.dht(), silent.addr.port());
        silent.wait_for(2).await;
        assert_eq!(
            silent.sources(),
            vec![lab.e_addr; 2],
            "send and retransmission are from E"
        );
        let seen = silent.count();
        // when: the session is dropped
        drop(session);
        tokio::time::sleep(RETRY_WINDOW).await;
        // then: after the drop no retransmissions and no other traffic
        assert_eq!(silent.count(), seen, "retransmission after session drop");
    }

    #[tokio::test]
    async fn forward_inbound_via_after_destroy_is_silent() {
        // given: the node is destroyed and its actor stopped
        let lab = setup().await;
        let r = Remote::start().await;
        lab.a.destroy().await.expect("destroy");
        lab.wait_until_actor_closed().await;
        // when: a request from R comes via E
        lab.a.forward_inbound_via(
            r.addr,
            request_bytes(REQUEST_TID, lab.e_addr),
            lab.e.clone(),
        );
        // then: the call does not panic (the test reaches this line), no response
        assert!(
            r.recv_within(QUIET).await.is_none(),
            "destroyed node replied"
        );
    }
}

// ── setup and helpers ─────────────────────────────────────────────────────────

type Call = tokio::task::JoinHandle<Result<ResponseData, DhtError>>;

/// Node A with external socket E on `127.0.0.1`; E is read by a task that forwards datagrams to A.
struct Lab {
    rt: UdxRuntime,
    a: DhtHandle,
    e: UdxSocket,
    e_addr: SocketAddr,
}

impl Lab {
    /// A `COMMAND` request via E to `127.0.0.1:port` on behalf of `dht`, waits in its own task.
    fn when_requests_via_e(&self, dht: &DhtHandle, port: u16) -> Call {
        let (dht, e) = (dht.clone(), self.e.clone());
        tokio::spawn(async move { dht.request_via(user_params(), "127.0.0.1", port, e).await })
    }

    fn e_peer(&self) -> Ipv4Peer {
        peer(self.e_addr.port())
    }

    /// After `destroy` the actor leaves the loop and drops the command receiver.
    async fn wait_until_actor_closed(&self) {
        for _ in 0..100 {
            if self.a.local_port().await.is_err() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

async fn setup() -> Lab {
    let rt = UdxRuntime::new().expect("runtime");
    let a = live_node(&rt).await;
    let e = rt.create_socket().await.expect("socket E");
    e.bind("127.0.0.1:0".parse().expect("addr"))
        .await
        .expect("bind E");
    let e_addr = e.local_addr().await.expect("addr E");
    pump(&a, &e);
    Lab { rt, a, e, e_addr }
}

/// What the `Reuse` forwarder does: DHT datagrams from E go to the actor of A.
fn pump(dht: &DhtHandle, e: &UdxSocket) {
    let mut rx = e.recv_start().expect("recv_start");
    let (dht, e) = (dht.clone(), e.clone());
    tokio::spawn(async move {
        while let Some(d) = rx.recv().await {
            dht.forward_inbound_via(d.addr, d.data, e.clone());
        }
    });
}

async fn live_node(rt: &UdxRuntime) -> DhtHandle {
    let cfg = DhtConfig {
        bootstrap: vec![],
        ephemeral: Some(true),
        ..DhtConfig::default()
    };
    let (_task, handle) = spawn(rt, cfg).await.expect("spawn");
    handle
}

fn channel_handle() -> (DhtHandle, UnboundedReceiver<DhtCommand>) {
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let handle = DhtHandle {
        cmd_tx,
        wire: WireCounters::new(),
        table: Arc::new(Mutex::new(RoutingTable::new([0u8; 32]))),
        suspended: Arc::new(AtomicBool::new(false)),
        session_id: None,
        mode: tokio::sync::watch::channel(super::DhtMode::default()).1,
    };
    (handle, cmd_rx)
}

fn params() -> UserRequestParams {
    UserRequestParams {
        command: COMMAND,
        ..UserRequestParams::default()
    }
}

fn user_params() -> UserRequestParams {
    UserRequestParams {
        command: COMMAND,
        value: Some(b"ping".to_vec()),
        ..UserRequestParams::default()
    }
}

/// Polls the future once or twice and drops it.
async fn poll_and_drop<F: std::future::Future>(fut: F) {
    let _ = tokio::time::timeout(Duration::from_millis(20), fut).await;
}

async fn within<F: std::future::Future>(fut: F) -> F::Output {
    tokio::time::timeout(WAIT, fut).await.expect("timed out")
}

fn peer(port: u16) -> Ipv4Peer {
    Ipv4Peer {
        host: "127.0.0.1".into(),
        port,
    }
}

/// A user `COMMAND` request; `to` is the receiver address (the sender writes it into the frame).
fn request_bytes(tid: u16, to: SocketAddr) -> Vec<u8> {
    let req = messages::Request {
        tid,
        to: peer(to.port()),
        id: None,
        token: None,
        internal: false,
        command: COMMAND,
        target: None,
        value: Some(b"hello".to_vec()),
    };
    messages::encode_request_to_bytes(&req).expect("encode request")
}

fn decode_response(data: &[u8]) -> messages::Response {
    match messages::decode_message(data).expect("decode") {
        Message::Response(res) => res,
        Message::Request(_) => panic!("expected a response, got a request"),
    }
}

/// External peer R: sends requests, reads responses and sees the sender address.
struct Remote {
    sock: UdpSocket,
    addr: SocketAddr,
}

impl Remote {
    async fn start() -> Self {
        let sock = UdpSocket::bind("127.0.0.1:0").await.expect("bind R");
        let addr = sock.local_addr().expect("addr R");
        Self { sock, addr }
    }

    async fn send_request(&self, to: SocketAddr, tid: u16) {
        let data = request_bytes(tid, to);
        self.sock.send_to(&data, to).await.expect("send");
    }

    async fn recv_within(&self, d: Duration) -> Option<(Vec<u8>, SocketAddr)> {
        let mut buf = vec![0u8; 2048];
        let (n, src) = tokio::time::timeout(d, self.sock.recv_from(&mut buf))
            .await
            .ok()?
            .expect("recv");
        Some((buf[..n].to_vec(), src))
    }
}

type Log = Arc<Mutex<Vec<(Vec<u8>, SocketAddr)>>>;

/// A UDP node that stays silent and records datagrams with the sender address.
struct SilentPeer {
    addr: SocketAddr,
    seen: Log,
}

impl SilentPeer {
    async fn start() -> Self {
        let sock = UdpSocket::bind("127.0.0.1:0").await.expect("bind");
        let addr = sock.local_addr().expect("addr");
        let seen = Log::default();
        let log = Arc::clone(&seen);
        tokio::spawn(async move {
            let mut buf = vec![0u8; 2048];
            while let Ok((n, src)) = sock.recv_from(&mut buf).await {
                log.lock().expect("log").push((buf[..n].to_vec(), src));
            }
        });
        Self { addr, seen }
    }

    fn count(&self) -> usize {
        self.seen.lock().expect("log").len()
    }

    fn sources(&self) -> Vec<SocketAddr> {
        let seen = self.seen.lock().expect("log");
        seen.iter().map(|(_, src)| *src).collect()
    }

    /// Waits until at least `n` datagrams arrive (the first retransmission is after 1000 ms).
    async fn wait_for(&self, n: usize) {
        for _ in 0..300 {
            if self.count() >= n {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("got {} datagrams out of {n}", self.count());
    }
}
