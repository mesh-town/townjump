//! Cluster tests of the server's relay use: it joins the client's relay, prefers its own,
//! stays off the relay for open or direct clients, and ignores a relay that is not there.

use std::net::SocketAddr;
use std::time::Duration;

use tokio::sync::oneshot;

use crate::blind_relay::BlindRelayServer;
use crate::crypto::hash;
use crate::hyperdht::{ConnectOpts, HolepunchFilter, KeyPair, PeerConnection, RelayThrough};
use crate::hyperdht_messages::{FIREWALL_OPEN, FIREWALL_UNKNOWN, RelayThroughInfo};
use crate::relay_service::{RelayServiceConfig, RelayServiceHandle, run_relay_server};
use crate::server::ServerOpts;
use crate::server::test_net::{
    Cluster, Node, Served, cluster, echo, hello_relay, key, lan_host, member_on, node,
    primary_port, raw_handshake, roundtrip, serve, sha256, within,
};

/// A blind-relay node of the cluster.
pub(super) struct Relay {
    pub(super) pk: [u8; 32],
    pub(super) stats: BlindRelayServer,
    node: Node,
    _service: RelayServiceHandle,
}

pub(super) async fn relay(cl: &Cluster) -> Relay {
    let mut node = member_on(cl, &lan_host()).await;
    let kp = KeyPair::generate();
    let events = std::mem::replace(&mut node.events, tokio::sync::mpsc::unbounded_channel().1);
    let (rt, handle) = (node.rt.handle(), node.handle.clone());
    let (stats, service) = run_relay_server(
        rt,
        handle,
        kp.clone(),
        events,
        RelayServiceConfig::default(),
    );
    for _ in 0..100 {
        if matches!(
            cl.nodes[0].handle.find_peer(hash(&kp.public_key)).await,
            Ok(Some(_))
        ) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    Relay {
        pk: kp.public_key,
        stats,
        node,
        _service: service,
    }
}

fn matched(r: &Relay) -> u64 {
    r.stats.stats().pairings_matched
}

/// Accepts one connection, reports its remote address, echoes it.
fn echo_once(served: Served) -> oneshot::Receiver<Option<SocketAddr>> {
    let (tx, rx) = oneshot::channel();
    tokio::spawn(async move {
        let mut served = served;
        let Some(conn) = served.incoming.recv().await else {
            return;
        };
        let _ = tx.send(conn.remote_addr);
        echo(conn).await;
    });
    rx
}

fn client_opts(relay: Option<[u8; 32]>, punch: bool) -> ConnectOpts {
    ConnectOpts {
        local_connection: false,
        relay_through: relay.map(RelayThrough::Key),
        holepunch: (!punch).then(|| HolepunchFilter::new(|_| false)),
        ..ConnectOpts::default()
    }
}

pub(super) async fn dial(cl: &Cluster, o: ConnectOpts, tag: u8) -> PeerConnection {
    let client = node(cl, true).await;
    let me = key(tag);
    let found = client
        .handle
        .connect_with_options(&me, key(1).public_key, &[], &client.rt, o);
    let conn = tokio::time::timeout(Duration::from_secs(20), found).await;
    std::mem::forget(client);
    conn.expect("connect in 20 s").expect("connected")
}

async fn echo_64k(conn: PeerConnection) {
    let data: Vec<u8> = (0..64 * 1024).map(|i| (i % 251) as u8).collect();
    assert_eq!(roundtrip(conn, data.clone()).await, sha256(&data));
}

#[tokio::test(flavor = "multi_thread")]
async fn server_joins_clients_relay_as_non_initiator() {
    within(async {
        // given: a firewalled server without a relay of its own, the client names relay R
        let cl = cluster(4).await;
        let r = relay(&cl).await;
        let opts = ServerOpts {
            holepunch: Some(HolepunchFilter::new(|_| false)),
            ..ServerOpts::default()
        };
        let seen = echo_once(serve(node(&cl, true).await, &key(1), opts).await);
        // when: the client connects (no direct path) and echoes 64 KiB
        echo_64k(dial(&cl, client_opts(Some(r.pk), false), 2).await).await;
        // then: the server's connection runs through R, which made one pairing
        let relay_addr = SocketAddr::new(
            lan_host().parse().expect("lan ip"),
            primary_port(&r.node.handle).await,
        );
        assert_eq!(seen.await.expect("server saw it"), Some(relay_addr));
        assert_eq!(matched(&r), 1);
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn both_announce_server_relay_wins() {
    within(async {
        // given: the client names R1, the server R2
        let cl = cluster(4).await;
        let (r1, r2) = (relay(&cl).await, relay(&cl).await);
        let opts = ServerOpts {
            relay_through: Some(RelayThrough::Key(r2.pk)),
            ..ServerOpts::default()
        };
        let _seen = echo_once(serve(node(&cl, true).await, &key(1), opts).await);
        // when: they connect and echo
        echo_64k(dial(&cl, client_opts(Some(r1.pk), false), 2).await).await;
        // then: the server's relay paired them
        assert_eq!((matched(&r2), matched(&r1)), (1, 0));
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn open_or_direct_client_gets_no_relay() {
    within(async {
        // given: a server announcing R and two raw clients that ask for a relay too
        let cl = cluster(4).await;
        let r = relay(&cl).await;
        let opts = ServerOpts {
            relay_through: Some(RelayThrough::Key(r.pk)),
            ..ServerOpts::default()
        };
        let served = serve(node(&cl, true).await, &key(1), opts).await;
        let prober = node(&cl, false).await;
        let ask = Some(RelayThroughInfo {
            version: 1,
            public_key: [7; 32],
            token: [8; 32],
        });
        let port = primary_port(&served.handle).await;
        let direct = crate::messages::Ipv4Peer {
            host: "127.0.0.1".into(),
            port,
        };
        // when: one open client comes through a node, one unknown client straight to the server
        let cases = [
            (FIREWALL_OPEN, 2, &cl.addrs[0]),
            (FIREWALL_UNKNOWN, 3, &direct),
        ];
        for (firewall, tag, to) in cases {
            let (mut nw, noise) = hello_relay(
                &key(tag),
                key(1).public_key,
                firewall,
                Some(41),
                vec![],
                ask.clone(),
            );
            let reply = raw_handshake(&prober, key(1).public_key, &noise, to).await;
            // then: the reply still announces R
            let payload = nw.recv(&reply.expect("reply").noise).expect("payload");
            assert_eq!(payload.relay_through.map(|i| i.public_key), Some(r.pk));
        }
        // then: the server never went to the relay
        tokio::time::sleep(Duration::from_secs(3)).await;
        assert_eq!(r.stats.stats().sessions_accepted, 0);
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn unreachable_relay_does_not_block_direct() {
    within(async {
        // given: a server whose relay does not exist, a client that can punch
        let cl = cluster(4).await;
        let opts = ServerOpts {
            relay_through: Some(RelayThrough::Key([0x77; 32])),
            ..ServerOpts::default()
        };
        let _seen = echo_once(serve(node(&cl, true).await, &key(1), opts).await);
        // when: the client connects
        let started = std::time::Instant::now();
        let conn = dial(&cl, client_opts(None, true), 2).await;
        // then: it is quick, direct, and not through a cluster node
        assert!(started.elapsed() < Duration::from_secs(10));
        assert!(conn.relay_path().is_none());
        let addr = conn.remote_addr.expect("remote addr");
        assert!(cl.addrs.iter().all(|a| a.port != addr.port()));
    })
    .await;
}
