// JS: dht-rpc/lib/io.js
//! Tests for IO through the external socket E: a request goes out, is retransmitted and waits for the window, with E and without a node
//! id. R is an ordinary `tokio::net::UdpSocket`: the sender address shows which socket the
//! datagram left from. Response via E and incoming request via E: `io_via_inbound_tests.rs` (module
//! `inbound`). Related: `io_retry_tests.rs` (ordinary path retransmissions), `io_cancel_tests.rs`.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use libudx::{Datagram, UdxRuntime, UdxSocket};
use tokio::net::UdpSocket;
use tokio::time::Instant;

use super::retry_tests::{DEAD_PORT, NODE_ID, peer};
use super::*;
use crate::messages::{self, Ipv4Peer, Message};
use crate::routing_table::RoutingTable;

const COMMAND: u64 = 7;
const REQUEST_TID: u16 = 41;
const ERROR_INVALID_TOKEN: u64 = 2;
/// How long we wait for a datagram that must arrive.
const WAIT: Duration = Duration::from_secs(2);
/// How long we wait to be sure no datagram will come.
const QUIET: Duration = Duration::from_millis(200);

#[path = "io_via_inbound_tests.rs"]
mod inbound;

#[path = "io_via_log_tests.rs"]
mod log;

#[path = "io_server_arrival_tests.rs"]
mod server_arrival;

mod request_via_e {
    use super::*;

    mod first_send {
        use super::*;

        #[tokio::test]
        async fn leaves_from_e_without_node_id() {
            // given: a node with an id (not ephemeral, not firewalled), external socket E, receiver R
            let mut f = setup(Opts::default()).await;
            // when: a request via E to R
            let tid = f.when_request_via(None);
            // then: R sees the request from the address of E, our tid, no node id
            let (data, from) = f.r_receives().await;
            assert_eq!(from, f.e_addr, "first send did not go out from E");
            let req = decode_request(&data);
            assert_eq!(req.tid, tid);
            assert_eq!(req.command, COMMAND);
            assert_eq!(req.id, None, "node id is not sent via E (JS io.js:488)");
        }
    }

    mod expired_timer {
        use super::*;

        #[tokio::test]
        async fn each_of_three_retries_leaves_from_e() {
            // given: a request via E was sent
            let mut f = setup(Opts::default()).await;
            let tid = f.when_request_via(None);
            let (first, _) = f.r_receives().await;
            for n in 1..=3 {
                // when: the timer expires
                let (retries, timeouts) = f.when_expired(tid);
                // then: one retransmission, the same request and also from E
                assert_eq!(retries.len(), 1, "expiry {n}");
                assert!(timeouts.is_empty(), "expiry {n}: no final event yet");
                let (data, from) = f.r_receives().await;
                assert_eq!(from, f.e_addr, "retransmission {n} did not go out from E");
                assert_eq!(data, first, "retransmission {n}: same request");
            }
        }

        #[tokio::test]
        async fn fourth_expiry_is_the_final_timeout_and_removes_the_entry() {
            // given: the first send and three retransmissions behind
            let mut f = setup(Opts::default()).await;
            let tid = f.when_request_via(None);
            for _ in 0..3 {
                f.when_expired(tid);
            }
            for _ in 0..4 {
                f.r_receives().await;
            }
            // when: the fourth timer expires
            let (retries, timeouts) = f.when_expired(tid);
            // then: final TimeoutEvent, no retransmission, entry removed
            assert!(retries.is_empty(), "no retransmission after three");
            assert_eq!(timeouts.len(), 1);
            assert_eq!(timeouts[0].tid, tid);
            assert!(f.io.inflight.is_empty(), "entry removed from inflight");
            f.r_gets_nothing().await;
        }
    }

    mod window_full {
        use super::*;

        #[tokio::test]
        async fn queued_request_leaves_from_e_after_drain() {
            // given: a one-slot window is taken by an ordinary request
            let mut f = setup(Opts { max_window: 1 }).await;
            f.when_plain_request_fills_the_window();
            // when: a request via E
            let tid = f.when_request_via(None);
            // then: it waits in the queue, R received nothing
            assert!(
                f.io.pending.iter().any(|p| p.tid == tid),
                "request in pending"
            );
            f.r_gets_nothing().await;
            // when: the window freed up
            f.io.drain();
            // then: the request left the queue and reached R from the address of E
            assert!(f.io.pending.is_empty(), "queue drained");
            let (data, from) = f.r_receives().await;
            assert_eq!(from, f.e_addr, "queued request did not go out from E");
            assert_eq!(decode_request(&data).tid, tid);
        }
    }

    mod owner_cancelled {
        use super::*;

        #[tokio::test]
        async fn drops_the_entry_and_stops_retries() {
            // given: a request via E with a session owner was sent; its timer expired
            let mut f = setup(Opts::default()).await;
            let tid = f.when_request_via(Some(Owner::Session(7)));
            f.r_receives().await;
            f.expire(tid);
            // when: the session is cancelled
            let gone = f.io.cancel_owner(Owner::Session(7));
            // then: entry removed, no retransmission, no timeout, R is silent
            assert_eq!(gone, vec![tid]);
            let (retries, timeouts) = f.io.check_timeouts_and_retries();
            assert!(retries.is_empty(), "retransmission of a cancelled request");
            assert!(timeouts.is_empty(), "timeout of a cancelled request");
            assert!(f.io.inflight.is_empty());
            f.r_gets_nothing().await;
        }
    }

    mod refused {
        use super::*;

        #[tokio::test]
        async fn closed_io_gives_none_and_sends_nothing() {
            // given: IO is frozen
            let mut f = setup(Opts::default()).await;
            f.io.suspend();
            // when: a request via E
            let tid = f.create_via(f.r_peer(), None);
            // then: None, no entry, R is silent
            assert_eq!(tid, None);
            assert!(f.io.inflight.is_empty());
            f.r_gets_nothing().await;
        }

        #[tokio::test]
        async fn unparsable_address_gives_none() {
            // given: a destination without a parseable address
            let mut f = setup(Opts::default()).await;
            let nowhere = Ipv4Peer {
                host: "not-an-ip".into(),
                port: 1,
            };
            // when: a request via E
            let tid = f.create_via(nowhere, None);
            // then: None, no entry
            assert_eq!(tid, None);
            assert!(f.io.inflight.is_empty());
        }
    }

    mod wire_counters {
        use super::*;

        #[tokio::test]
        async fn sent_bytes_are_counted() {
            let mut f = setup(Opts::default()).await;
            // when: a request went out via E
            f.when_request_via(None);
            // then: bytes_sent equals the size of the datagram that reached R
            let (data, _) = f.r_receives().await;
            assert_eq!(f.io.wire.snapshot().0, data.len() as u64);
        }
    }
}

// ── setup and helpers ─────────────────────────────────────────────────────────

/// Fixture preconditions. The node always has an id (`ephemeral: false, firewalled: false`): otherwise `id`
/// would not be sent even without the "via E without id" rule.
struct Opts {
    max_window: usize,
}

impl Default for Opts {
    fn default() -> Self {
        Self { max_window: 80 }
    }
}

struct Fixture {
    io: Io,
    /// External socket E: foreign to IO.
    e: UdxSocket,
    e_addr: SocketAddr,
    /// Remote side R.
    r: UdpSocket,
    r_addr: SocketAddr,
    /// The runtime is kept for the whole test; the field is last so the sockets close first.
    _rt: UdxRuntime,
}

impl Fixture {
    fn r_peer(&self) -> Ipv4Peer {
        peer(self.r_addr.port())
    }

    fn create_via(&mut self, to: Ipv4Peer, owner: Option<Owner>) -> Option<u16> {
        let params = RequestParams {
            to,
            command: COMMAND,
            ..RequestParams::default()
        };
        self.io.create_request_via(params, owner, self.e.clone())
    }

    /// A request via E to R; returns its tid.
    fn when_request_via(&mut self, owner: Option<Owner>) -> u16 {
        self.create_via(self.r_peer(), owner)
            .expect("create_request_via")
    }

    /// An ordinary request (from the IO sockets) takes the last window slot.
    fn when_plain_request_fills_the_window(&mut self) {
        let params = RequestParams {
            to: peer(DEAD_PORT),
            internal: true,
            command: 1,
            ..RequestParams::default()
        };
        self.io.create_request(params).expect("create_request");
        assert!(self.io.congestion.is_full(), "window is full");
    }

    /// The deadline of request `tid` has passed.
    fn expire(&mut self, tid: u16) {
        let entry = self.io.inflight.iter_mut().find(|e| e.tid == tid);
        entry.expect("an entry with this tid exists").deadline = Instant::now();
    }

    fn when_expired(&mut self, tid: u16) -> (Vec<RetryEvent>, Vec<TimeoutEvent>) {
        self.expire(tid);
        self.io.check_timeouts_and_retries()
    }

    /// A datagram from R is accepted via E.
    fn arrive_via_e(&mut self, from: SocketAddr, data: Vec<u8>) -> Option<IoEvent> {
        self.io.handle_inbound_via(from, data, self.e.clone())
    }

    fn when_response_arrives_via_e(&mut self, tid: u16, to: Ipv4Peer) -> Option<IoEvent> {
        self.arrive_via_e(self.r_addr, response_bytes(tid, to))
    }

    fn when_request_arrives_via_e(&mut self, token: Option<[u8; 32]>) -> Option<IoEvent> {
        self.arrive_via_e(self.r_addr, request_bytes(token))
    }

    /// A request from R is accepted on the IO socket of kind `kind` (ordinary path).
    fn when_request_arrives_on(&mut self, kind: SocketKind) -> Option<IoEvent> {
        let datagram = Datagram {
            data: request_bytes(None),
            addr: self.r_addr,
        };
        self.io.process_datagram(datagram, kind)
    }

    /// The next datagram that reached R: bytes and sender address.
    async fn r_receives(&self) -> (Vec<u8>, SocketAddr) {
        let mut buf = vec![0u8; 2048];
        let got = tokio::time::timeout(WAIT, self.r.recv_from(&mut buf)).await;
        let (n, from) = got.expect("R waited for a datagram").expect("recv_from");
        buf.truncate(n);
        (buf, from)
    }

    async fn r_gets_nothing(&self) {
        let mut buf = vec![0u8; 2048];
        let got = tokio::time::timeout(QUIET, self.r.recv_from(&mut buf)).await;
        assert!(got.is_err(), "R must not receive datagrams");
    }
}

/// IO on loopback, external socket E and receiver R.
async fn setup(opts: Opts) -> Fixture {
    let rt = UdxRuntime::new().expect("runtime");
    let table = Arc::new(Mutex::new(RoutingTable::new(NODE_ID)));
    let config = IoConfig {
        host: "127.0.0.1".into(),
        port: 0,
        max_window: opts.max_window,
        firewalled: false,
        ephemeral: false,
    };
    let io = Io::bind(&rt, table, config).await.expect("bind");
    let e = rt.create_socket().await.expect("create E");
    e.bind("127.0.0.1:0".parse().expect("addr"))
        .await
        .expect("bind E");
    let e_addr = addr_of(&e).await;
    let r = UdpSocket::bind("127.0.0.1:0").await.expect("bind R");
    let r_addr = r.local_addr().expect("R addr");
    Fixture {
        io,
        e,
        e_addr,
        r,
        r_addr,
        _rt: rt,
    }
}

/// Socket address (sockets are compared by it).
async fn addr_of(socket: &UdxSocket) -> SocketAddr {
    socket.local_addr().await.expect("local_addr")
}

fn request_bytes(token: Option<[u8; 32]>) -> Vec<u8> {
    let req = messages::Request {
        tid: REQUEST_TID,
        to: peer(1),
        id: None,
        token,
        internal: false,
        command: COMMAND,
        target: None,
        value: None,
    };
    messages::encode_request_to_bytes(&req).expect("encode request")
}

fn response_bytes(tid: u16, to: Ipv4Peer) -> Vec<u8> {
    let res = messages::Response {
        tid,
        to,
        id: None,
        token: None,
        closer_nodes: vec![],
        error: 0,
        value: None,
    };
    messages::encode_response_to_bytes(&res).expect("encode response")
}

fn decode_request(data: &[u8]) -> messages::Request {
    match messages::decode_message(data).expect("decode") {
        Message::Request(req) => req,
        Message::Response(_) => panic!("expected a request, got a response"),
    }
}

fn decode_response(data: &[u8]) -> messages::Response {
    match messages::decode_message(data).expect("decode") {
        Message::Response(res) => res,
        Message::Request(_) => panic!("expected a response, got a request"),
    }
}

/// Incoming request from an IO event.
fn incoming(event: Option<IoEvent>) -> IncomingRequest {
    match event {
        Some(IoEvent::IncomingRequest(req)) => req,
        _ => panic!("expected IoEvent::IncomingRequest"),
    }
}
