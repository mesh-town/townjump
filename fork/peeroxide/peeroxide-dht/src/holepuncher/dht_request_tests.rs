//! End-to-end tests for DHT requests that reach the socket of a [`Holepuncher`] before connect:
//! an internal PING is answered from that same socket, everything else stays unanswered, and the
//! pinger does not end up in the routing table unless its id matches its address.
//!
//! Uses only API that exists without the request lane, apart from the `serve_dht_requests` call
//! made inside `auto_sample` / `start_background_sampling`.

use std::net::SocketAddr;
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::sync::mpsc;

use crate::holepuncher::{HolepunchEvent, Holepuncher};
use crate::hyperdht_messages::FIREWALL_UNKNOWN;
use crate::messages::{self, Command, Ipv4Peer, Message, Request, Response};
use crate::peer::{self, NodeId};
use crate::rpc::{DhtConfig, DhtHandle};
use crate::socket_pool::SocketPool;

const ANSWER_WAIT: Duration = Duration::from_secs(2);
const SILENCE_WAIT: Duration = Duration::from_millis(500);

mod server_puncher {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ping_before_connect_is_answered_from_puncher_socket() {
        // given
        let lab = setup(Role::Server).await;
        // when
        lab.remote.send(ping(7, lab.puncher_addr, None)).await;
        // then
        lab.then_answered_from_puncher_socket(7).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn user_request_with_command_zero_is_not_answered() {
        // given
        let lab = setup(Role::Server).await;
        // when: command 0 is PING only for internal requests
        lab.remote
            .send(raw_request(8, lab.puncher_addr, false, 0, None))
            .await;
        // then
        lab.then_silence().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn find_node_is_not_answered() {
        // given
        let lab = setup(Role::Server).await;
        // when: the answer would expose the routing table through the puncher socket
        let target: NodeId = rand::random();
        let find_node = Command::FindNode as u64;
        lab.remote
            .send(raw_request(
                9,
                lab.puncher_addr,
                true,
                find_node,
                Some(target),
            ))
            .await;
        // then
        lab.then_silence().await;
    }
}

mod client_puncher {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ping_before_connect_is_answered_from_client_puncher_socket() {
        // given
        let lab = setup(Role::Client).await;
        // when
        lab.remote.send(ping(7, lab.puncher_addr, None)).await;
        // then
        lab.then_answered_from_puncher_socket(7).await;
    }
}

mod routing_table {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pinger_without_id_stays_out_of_table() {
        // given
        let lab = setup(Role::Server).await;
        assert_eq!(lab.table_size().await, 0, "table starts empty");
        // when
        lab.remote.send(ping(7, lab.puncher_addr, None)).await;
        lab.then_answered_from_puncher_socket(7).await;
        // then
        assert_eq!(lab.table_size().await, 0, "pinger without id was added");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pinger_with_foreign_id_stays_out_of_table() {
        // given
        let lab = setup(Role::Server).await;
        // when: the id is not the one derived from the sender address
        let foreign: NodeId = rand::random();
        lab.remote
            .send(ping(7, lab.puncher_addr, Some(foreign)))
            .await;
        lab.then_answered_from_puncher_socket(7).await;
        // then
        assert_eq!(
            lab.table_size().await,
            0,
            "pinger with a foreign id was added"
        );
    }

    /// Control for the two tests above: the same node does add a pinger whose id matches its
    /// address, so "table stays empty" there is not a property of the instrument. The node is
    /// ephemeral like in the other tests; the table is filled for ephemeral nodes too.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pinger_with_matching_id_is_added() {
        // given
        let lab = setup(Role::Server).await;
        // when
        let id = peer::peer_id("127.0.0.1", lab.remote.port());
        lab.remote.send(ping(7, lab.puncher_addr, Some(id))).await;
        lab.then_answered_from_puncher_socket(7).await;
        // then
        assert_eq!(
            lab.table_size().await,
            1,
            "pinger with a matching id was not added"
        );
    }
}

enum Role {
    Server,
    Client,
}

/// A live DHT node, a puncher of the given role (before connect) and an outside UDP peer.
struct Lab {
    _rt: libudx::UdxRuntime,
    _node: tokio::task::JoinHandle<Result<(), crate::rpc::DhtError>>,
    dht: DhtHandle,
    _puncher: Holepuncher,
    _events: mpsc::UnboundedReceiver<HolepunchEvent>,
    puncher_addr: SocketAddr,
    remote: Remote,
}

impl Lab {
    async fn table_size(&self) -> usize {
        self.dht.table_size().await.expect("table size")
    }

    /// The outside peer gets a response with `tid` and no node id, sent from the puncher socket.
    async fn then_answered_from_puncher_socket(&self, tid: u16) {
        let (data, src) = self
            .remote
            .recv_within(ANSWER_WAIT)
            .await
            .expect("no answer within 2 s");
        let res = decode_response(&data);
        assert_eq!(res.tid, tid, "answer carries the request tid");
        assert_eq!(res.id, None, "a puncher-socket answer carries no node id");
        assert_eq!(src, self.puncher_addr, "answer left from another socket");
    }

    /// Not a single datagram, error answers included.
    async fn then_silence(&self) {
        let got = self.remote.recv_within(SILENCE_WAIT).await;
        assert!(got.is_none(), "unexpected datagram: {got:?}");
    }
}

async fn setup(role: Role) -> Lab {
    let rt = libudx::UdxRuntime::new().expect("runtime");
    let (node, dht) = live_node(&rt).await;
    let is_initiator = matches!(role, Role::Client);
    let pool = SocketPool::new("127.0.0.1".to_string());
    let (tx, events) = mpsc::unbounded_channel();
    let mut puncher = Holepuncher::new(&pool, &rt, true, is_initiator, FIREWALL_UNKNOWN, tx)
        .await
        .expect("puncher");
    match role {
        Role::Server => {
            puncher.auto_sample(&dht).await;
        }
        Role::Client => puncher.start_background_sampling(&dht),
    }
    let socket = puncher
        .primary_socket()
        .expect("primary socket")
        .socket
        .clone();
    let puncher_addr = socket.local_addr().await.expect("puncher addr");
    Lab {
        _rt: rt,
        _node: node,
        dht,
        _puncher: puncher,
        _events: events,
        puncher_addr,
        remote: Remote::start().await,
    }
}

async fn live_node(
    rt: &libudx::UdxRuntime,
) -> (
    tokio::task::JoinHandle<Result<(), crate::rpc::DhtError>>,
    DhtHandle,
) {
    let cfg = DhtConfig {
        bootstrap: vec![],
        host: "127.0.0.1".to_string(),
        ephemeral: Some(true),
        firewalled: true,
        ..DhtConfig::default()
    };
    crate::rpc::spawn(rt, cfg).await.expect("spawn dht")
}

fn raw_request(
    tid: u16,
    to: SocketAddr,
    internal: bool,
    command: u64,
    target: Option<NodeId>,
) -> Request {
    Request {
        tid,
        to: Ipv4Peer {
            host: "127.0.0.1".into(),
            port: to.port(),
        },
        id: None,
        token: None,
        internal,
        command,
        target,
        value: None,
    }
}

fn ping(tid: u16, to: SocketAddr, id: Option<NodeId>) -> Request {
    Request {
        id,
        ..raw_request(tid, to, true, Command::Ping as u64, None)
    }
}

fn decode_response(data: &[u8]) -> Response {
    match messages::decode_message(data).expect("decode") {
        Message::Response(res) => res,
        Message::Request(_) => panic!("expected a response, got a request"),
    }
}

/// The outside peer: a plain UDP socket that sends requests and reads whatever comes back.
struct Remote {
    sock: UdpSocket,
}

impl Remote {
    async fn start() -> Self {
        let sock = UdpSocket::bind("127.0.0.1:0").await.expect("bind remote");
        Self { sock }
    }

    fn port(&self) -> u16 {
        self.sock.local_addr().expect("remote addr").port()
    }

    /// Sends `req` to the address written in its `to` field (loopback, that port).
    async fn send(&self, req: Request) {
        let dest = SocketAddr::from(([127, 0, 0, 1], req.to.port));
        let data = messages::encode_request_to_bytes(&req).expect("encode request");
        self.sock.send_to(&data, dest).await.expect("send");
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
