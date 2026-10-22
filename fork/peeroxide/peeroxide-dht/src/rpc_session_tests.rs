// JS: dht-rpc/index.js Session, _autoDestroySession
//! Tests for `Session` and the implicit session of `DhtHandle::{find_node, query, request}`: what goes into the
//! command channel and what a live node sees when the future is dropped. Execution: `node_cancel_tests.rs`.

use std::net::SocketAddr;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use libudx::UdxRuntime;
use tokio::net::UdpSocket;
use tokio::sync::mpsc::{self, UnboundedReceiver};

use super::command::DhtCommand;
use super::{DhtConfig, DhtHandle, UserRequestParams, spawn};
use crate::io::WireCounters;
use crate::peer::{NodeId, peer_id};
use crate::routing_table::{Node, RoutingTable};

pub(super) fn channel_handle() -> (DhtHandle, UnboundedReceiver<DhtCommand>) {
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
        command: 1,
        ..UserRequestParams::default()
    }
}

/// Polls the future once or twice and drops it.
async fn poll_and_drop<F: std::future::Future>(fut: F) {
    let _ = tokio::time::timeout(Duration::from_millis(20), fut).await;
}

mod channel {
    use super::*;

    #[tokio::test]
    async fn dropping_a_session_sends_destroy_with_its_id() {
        // given: a session
        let (h, mut rx) = channel_handle();
        let s = h.session();
        let id = s.id();
        // when: the session is dropped
        drop(s);
        // then: the channel has DestroySession with its id
        match rx.try_recv() {
            Ok(DhtCommand::DestroySession { id: got }) => assert_eq!(got, id),
            _ => panic!("expected DestroySession"),
        }
    }

    #[tokio::test]
    async fn destroy_keeps_the_session_usable() {
        // given: a session after destroy
        let (h, mut rx) = channel_handle();
        let s = h.session();
        s.destroy();
        assert!(matches!(
            rx.try_recv(),
            Ok(DhtCommand::DestroySession { .. })
        ));
        // when: a request goes through it
        poll_and_drop(s.dht().request(params(), "127.0.0.1", 9)).await;
        // then: it goes out with the session id
        match rx.try_recv() {
            Ok(DhtCommand::Request { session, .. }) => assert_eq!(session, Some(s.id())),
            _ => panic!("expected Request"),
        }
    }

    #[tokio::test]
    async fn each_session_gets_a_new_id() {
        let (h, _rx) = channel_handle();
        let (a, b) = (h.session(), h.session());
        assert_ne!(a.id(), b.id());
        assert_ne!(a.id(), 0);
        assert_ne!(b.id(), 0);
    }

    #[tokio::test]
    async fn unscoped_calls_run_in_an_implicit_session_destroyed_with_the_future() {
        // given: a handle without a session
        let (h, mut rx) = channel_handle();
        // when: request and find_node are polled and dropped
        poll_and_drop(h.request(params(), "127.0.0.1", 9)).await;
        poll_and_drop(h.find_node(NodeId::default())).await;
        // then: each went out with session x, then DestroySession x
        for kind in ["request", "find_node"] {
            let used = match rx.try_recv() {
                Ok(DhtCommand::Request { session, .. } | DhtCommand::FindNode { session, .. }) => {
                    session.unwrap_or_else(|| panic!("{kind}: no session"))
                }
                _ => panic!("{kind}: expected a command with a session"),
            };
            match rx.try_recv() {
                Ok(DhtCommand::DestroySession { id }) => assert_eq!(id, used, "{kind}"),
                _ => panic!("{kind}: expected DestroySession"),
            }
        }
    }

    #[tokio::test]
    async fn scoped_call_sends_no_destroy_until_the_session_drops() {
        // given: a query via a session, the future dropped
        let (h, mut rx) = channel_handle();
        let s = h.session();
        let query = crate::rpc::UserQueryParams {
            target: [1u8; 32],
            command: 1,
            value: None,
            commit: false,
            concurrency: None,
        };
        poll_and_drop(s.dht().query(query)).await;
        assert!(matches!(rx.try_recv(), Ok(DhtCommand::Query { .. })));
        // then: no DestroySession while the session is alive
        assert!(rx.try_recv().is_err());
        // when: the session is dropped
        drop(s);
        // then: now there is one
        assert!(matches!(
            rx.try_recv(),
            Ok(DhtCommand::DestroySession { .. })
        ));
    }
}

type Log = Arc<Mutex<Vec<(Instant, Vec<u8>)>>>;

/// A UDP node that stays silent and records datagrams with the arrival time.
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
            while let Ok((n, _)) = sock.recv_from(&mut buf).await {
                log.lock()
                    .expect("log")
                    .push((Instant::now(), buf[..n].to_vec()));
            }
        });
        Self { addr, seen }
    }

    async fn first_arrival(&self) -> Instant {
        for _ in 0..200 {
            if let Some((t, _)) = self.seen.lock().expect("log").first() {
                return *t;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("datagram did not arrive");
    }

    /// How many datagrams byte-for-byte equal to the first arrived after `t`.
    fn repeats_of_first_after(&self, t: Instant) -> usize {
        let seen = self.seen.lock().expect("log");
        let first = &seen[0].1;
        seen.iter().filter(|(at, d)| *at > t && d == first).count()
    }
}

async fn live_node() -> (UdxRuntime, DhtHandle) {
    let rt = UdxRuntime::new().expect("runtime");
    let cfg = DhtConfig {
        bootstrap: vec![],
        ephemeral: Some(true),
        ..DhtConfig::default()
    };
    let (_task, handle) = spawn(&rt, cfg).await.expect("spawn");
    (rt, handle)
}

mod live_node {
    use super::*;

    #[tokio::test]
    async fn dropping_a_request_future_stops_its_retries() {
        // given: a request to a silent node, the first datagram arrived
        let (_rt, h) = live_node().await;
        let peer = SilentPeer::start().await;
        let port = peer.addr.port();
        let call = tokio::spawn(async move { h.request(params(), "127.0.0.1", port).await });
        let t = peer.first_arrival().await;
        // when: the future is dropped
        call.abort();
        let _ = call.await;
        tokio::time::sleep(Duration::from_millis(2500)).await;
        // then: no retransmissions
        assert_eq!(peer.repeats_of_first_after(t), 0);
    }

    #[tokio::test]
    async fn dropping_a_find_node_future_stops_its_retries() {
        // given: a silent node in the table, find_node to it
        let (_rt, h) = live_node().await;
        let peer = SilentPeer::start().await;
        let port = peer.addr.port();
        h.insert_node_for_test(Node {
            id: peer_id("127.0.0.1", port),
            host: "127.0.0.1".into(),
            port,
            token: None,
            added_tick: 0,
            seen_tick: 0,
            pinged_tick: 0,
            down_hints: 0,
        });
        let hh = h.clone();
        let call = tokio::spawn(async move { hh.find_node([7u8; 32]).await });
        let t = peer.first_arrival().await;
        // when: the future is dropped
        call.abort();
        let _ = call.await;
        tokio::time::sleep(Duration::from_millis(2500)).await;
        // then: no retransmissions
        assert_eq!(peer.repeats_of_first_after(t), 0);
    }
}
