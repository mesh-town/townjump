//! Tests for the DHT request lane of [`crate::socket_pool`]: classification of inbound datagrams,
//! the bounded queue while no DHT actor is attached, and release of the port with the `SocketRef`.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use tokio::net::UdpSocket;

use super::tests::port_frees_up;
use super::*;
use crate::messages::{self, Ipv4Peer, Request};
use crate::peer::NodeId;
use crate::rpc::DhtConfig;

const ANSWER_WAIT: Duration = Duration::from_secs(2);
const SILENCE_WAIT: Duration = Duration::from_millis(500);

mod classify {
    use super::*;

    #[test]
    fn classifies_internal_ping_as_dht_request() {
        let class = classify_inbound(&request_bytes(1, true, Command::Ping as u64, None));
        assert_eq!(class, InboundClass::DhtRequest);
    }

    #[test]
    fn classifies_user_command_zero_as_drop() {
        // Command 0 is PING only for internal requests.
        let class = classify_inbound(&request_bytes(1, false, 0, None));
        assert_eq!(class, InboundClass::Drop);
    }

    #[test]
    fn classifies_find_node_as_drop() {
        let target: NodeId = rand::random();
        let find_node = Command::FindNode as u64;
        let class = classify_inbound(&request_bytes(1, true, find_node, Some(target)));
        assert_eq!(class, InboundClass::Drop);
    }

    #[test]
    fn one_byte_and_udx_frame_keep_their_class() {
        assert_eq!(
            classify_inbound(&[0x01]),
            InboundClass::Holepunch,
            "one byte is a holepunch probe",
        );
        assert_eq!(
            classify_inbound(&[0xFF; 20]),
            InboundClass::UdxFrame,
            "20 bytes starting with 0xFF are a UDX frame",
        );
    }
}

mod serve {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ping_queued_before_serve_is_answered_after() {
        // given: a pool socket nobody serves yet
        let mut lab = setup().await;
        // when
        lab.remote_pings(7).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        lab.when_served();
        // then
        lab.then_answered_from_pool_socket(7).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn queue_holds_at_most_sixteen_pings() {
        // given
        let mut lab = setup().await;
        // when: 20 pings arrive before the actor is attached
        for tid in 1..=20 {
            lab.remote_pings(tid).await;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
        lab.when_served();
        // then
        let answers = lab.answers_within(Duration::from_secs(1)).await;
        assert_eq!(answers, DHT_REQUEST_QUEUE, "answers to 20 queued pings");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn one_byte_probe_still_reaches_holepunch_lane() {
        // given
        let mut lab = setup().await;
        lab.when_served();
        let mut probes = lab.socket_ref().take_holepunch_rx().expect("holepunch rx");
        // when
        lab.remote.sock.send_to(&[0], lab.addr).await.expect("send");
        // then
        let event = tokio::time::timeout(ANSWER_WAIT, probes.recv())
            .await
            .expect("no probe event within 2 s")
            .expect("holepunch lane closed");
        assert_eq!(event.addr, lab.remote.addr, "probe sender address");
        lab.then_silence().await;
    }
}

mod lifetime {
    use super::*;

    /// Fails if the request lane keeps a clone of the socket longer than the `SocketRef`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dropped_ref_releases_port_after_served_ping() {
        // given
        let mut lab = setup().await;
        lab.when_served();
        lab.remote_pings(7).await;
        lab.then_answered_from_pool_socket(7).await;
        // when
        let port = lab.addr.port();
        lab.drop_socket_ref();
        // then
        assert!(port_frees_up(port).await, "port still held after drop");
    }

    /// Fails if the queue outlives the `SocketRef`: every queued ping holds a socket clone.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dropped_ref_releases_port_with_unserved_pings() {
        // given: three pings queued, nobody serves
        let mut lab = setup().await;
        for tid in 1..=3 {
            lab.remote_pings(tid).await;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        // when
        let port = lab.addr.port();
        lab.drop_socket_ref();
        // then
        assert!(port_frees_up(port).await, "port still held after drop");
    }
}

/// A pool socket (no puncher), a live DHT node and an outside UDP peer.
struct Lab {
    _rt: libudx::UdxRuntime,
    _node: tokio::task::JoinHandle<std::result::Result<(), crate::rpc::DhtError>>,
    dht: DhtHandle,
    socket_ref: Option<SocketRef>,
    addr: SocketAddr,
    remote: Remote,
}

impl Lab {
    fn socket_ref(&mut self) -> &mut SocketRef {
        self.socket_ref.as_mut().expect("socket ref was dropped")
    }

    fn when_served(&mut self) {
        let dht = self.dht.clone();
        self.socket_ref().serve_dht_requests(&dht);
    }

    fn drop_socket_ref(&mut self) {
        drop(self.socket_ref.take());
    }

    async fn remote_pings(&self, tid: u16) {
        let data = request_bytes(tid, true, Command::Ping as u64, None);
        self.remote
            .sock
            .send_to(&data, self.addr)
            .await
            .expect("send");
    }

    async fn then_answered_from_pool_socket(&self, tid: u16) {
        let (data, src) = self
            .remote
            .recv_within(ANSWER_WAIT)
            .await
            .expect("no answer within 2 s");
        match messages::decode_message(&data).expect("decode") {
            Message::Response(res) => assert_eq!(res.tid, tid, "answer carries the request tid"),
            Message::Request(_) => panic!("expected a response, got a request"),
        }
        assert_eq!(src, self.addr, "answer left from another socket");
    }

    /// Number of datagrams that arrive at the outside peer before the deadline.
    async fn answers_within(&self, total: Duration) -> usize {
        let deadline = Instant::now() + total;
        let mut count = 0;
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            if self.remote.recv_within(left).await.is_some() {
                count += 1;
            }
        }
        count
    }

    async fn then_silence(&self) {
        let got = self.remote.recv_within(SILENCE_WAIT).await;
        assert!(got.is_none(), "unexpected datagram: {got:?}");
    }
}

async fn setup() -> Lab {
    let rt = libudx::UdxRuntime::new().expect("runtime");
    let cfg = DhtConfig {
        bootstrap: vec![],
        host: "127.0.0.1".to_string(),
        ephemeral: Some(true),
        firewalled: true,
        ..DhtConfig::default()
    };
    let (node, dht) = crate::rpc::spawn(&rt, cfg).await.expect("spawn dht");
    let pool = SocketPool::new("127.0.0.1".to_string());
    let socket_ref = pool.acquire(&rt).await.expect("acquire");
    let addr = socket_ref
        .socket
        .local_addr()
        .await
        .expect("pool socket addr");
    Lab {
        _rt: rt,
        _node: node,
        dht,
        socket_ref: Some(socket_ref),
        addr,
        remote: Remote::start().await,
    }
}

/// Encoded request; the receiver address in the frame is irrelevant for these tests.
fn request_bytes(tid: u16, internal: bool, command: u64, target: Option<NodeId>) -> Vec<u8> {
    let req = Request {
        tid,
        to: Ipv4Peer {
            host: "127.0.0.1".into(),
            port: 9000,
        },
        id: None,
        token: None,
        internal,
        command,
        target,
        value: None,
    };
    messages::encode_request_to_bytes(&req).expect("encode request")
}

/// The outside peer: a plain UDP socket.
struct Remote {
    sock: UdpSocket,
    addr: SocketAddr,
}

impl Remote {
    async fn start() -> Self {
        let sock = UdpSocket::bind("127.0.0.1:0").await.expect("bind remote");
        let addr = sock.local_addr().expect("remote addr");
        Self { sock, addr }
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
