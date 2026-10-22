//! Dropping a connect (or a won race) stops the network requests of the attempts: a silent
//! node gets no repeats of the handshake afterwards. JS reference: `hyperdht/lib/connect.js`
//! (`c.session.destroy()`). 127.0.0.1 only.
mod common;

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{cluster, serve_key, spawn_node, within};
use peeroxide_dht::hyperdht::KeyPair;
use peeroxide_dht::messages::Ipv4Peer;
use tokio::net::UdpSocket;

type Log = Arc<Mutex<Vec<(Instant, Vec<u8>)>>>;

/// A UDP node that never answers and records every datagram with its arrival time.
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

    fn peer(&self) -> Ipv4Peer {
        Ipv4Peer {
            host: self.addr.ip().to_string(),
            port: self.addr.port(),
        }
    }

    async fn first_arrival(&self) -> Instant {
        for _ in 0..300 {
            if let Some((t, _)) = self.seen.lock().expect("log").first() {
                return *t;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("no datagram arrived");
    }

    /// Datagrams byte-for-byte equal to the first one that arrived after `t`.
    fn repeats_of_first_after(&self, t: Instant) -> usize {
        let seen = self.seen.lock().expect("log");
        let first = &seen[0].1;
        seen.iter().filter(|(at, d)| *at > t && d == first).count()
    }
}

const SETTLE: Duration = Duration::from_millis(2500);

#[tokio::test]
async fn dropping_a_connect_stops_its_handshake() {
    within(async {
        let silent = SilentPeer::start().await;
        let client = spawn_node(vec![], true).await;
        let me = KeyPair::from_seed([0x71; 32]);
        let key = KeyPair::from_seed([0x72; 32]).public_key;
        let relays = [silent.peer()];
        let connect = client
            .handle
            .connect_with_nodes(&me, key, &relays, &client.rt);
        let t = tokio::select! {
            _ = connect => panic!("connect to a silent node cannot succeed this fast"),
            t = silent.first_arrival() => t,
        };
        tokio::time::sleep(SETTLE).await;
        assert_eq!(silent.repeats_of_first_after(t), 0);
    })
    .await;
}

#[tokio::test]
async fn dropping_connect_to_stops_its_handshake() {
    within(async {
        let silent = SilentPeer::start().await;
        let client = spawn_node(vec![], true).await;
        let me = KeyPair::from_seed([0x73; 32]);
        let key = KeyPair::from_seed([0x74; 32]).public_key;
        let connect = client.handle.connect_to(&me, key, silent.addr, &client.rt);
        let t = tokio::select! {
            _ = connect => panic!("connect_to a silent node cannot succeed this fast"),
            t = silent.first_arrival() => t,
        };
        tokio::time::sleep(SETTLE).await;
        assert_eq!(silent.repeats_of_first_after(t), 0);
    })
    .await;
}

#[tokio::test]
async fn a_won_connect_stops_the_losing_handshake() {
    within(async {
        let (_nodes, bootstrap) = cluster(4).await;
        let key = KeyPair::from_seed([0x75; 32]);
        let _served = serve_key(&bootstrap, &key, false).await;
        let silent = SilentPeer::start().await;
        let client = spawn_node(bootstrap, true).await;
        let me = KeyPair::from_seed([0x76; 32]);
        let relays = [silent.peer()];
        let conn = client
            .handle
            .connect_with_nodes(&me, key.public_key, &relays, &client.rt)
            .await;
        assert!(conn.is_ok(), "connect must succeed");
        let t = Instant::now();
        silent.first_arrival().await;
        tokio::time::sleep(SETTLE).await;
        assert_eq!(silent.repeats_of_first_after(t), 0);
    })
    .await;
}
