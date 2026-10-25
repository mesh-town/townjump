//! Connects of a client behind the server's own public address (a loopback cluster). The client
//! probes private candidates next to hole-punching; the first path that answers wins, and when
//! nothing answers the connect fails instead of returning a dead stream. A test lever replaces
//! the candidates with a silent socket and/or the advertised puncher port.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use tokio::time::timeout;

use crate::holepuncher::{HolepunchEvent, Holepuncher};
use crate::hyperdht::{
    ConnectOpts, HolepunchFilter, HyperDhtError, LAN_PROBE_DEADLINE, LanProbe, LanProbes,
    LanRewrite, PeerConnection,
};
use crate::hyperdht_messages::FIREWALL_UNKNOWN;
use crate::messages::Ipv4Peer;
use crate::server::ServerOpts;
use crate::server::test_net::{
    Cluster, Node, Served, cluster, echo, key, node, roundtrip, serve, settled_remote_address,
    sha256, within,
};
use crate::socket_pool::SocketPool;

mod client_behind_the_server_public_address {
    use super::*;

    #[tokio::test]
    async fn connects_with_the_advertised_candidates() {
        within(async {
            // given: both nodes behind one address, default connect options
            let mut fx = setup().await;
            // when: the client connects
            let client = fx.when_connect(ConnectOpts::default()).await;
            // then: connected, the server accepted the client's key and data flows
            let client = client.expect("connect over the advertised candidates");
            fx.then_server_accepted_and_data_flows(client).await;
        })
        .await;
    }

    #[tokio::test]
    async fn silent_first_candidate_gives_way_to_the_second() {
        within(async {
            // given: the first candidate is silent, the second is the advertised puncher; punch vetoed
            let mut fx = setup().await;
            let lever = probing(&[Candidate::Silent, Candidate::Advertised], veto());
            // when: the client connects
            let started = Instant::now();
            let client = fx.when_connect(lever.opts.clone()).await;
            let took = started.elapsed();
            // then: connected through the second candidate after the first one timed out
            let client = client.expect("connect through the second candidate");
            let port = lever.advertised.load(Ordering::SeqCst);
            assert_ne!(port, 0, "the server advertised no port");
            assert_eq!(client.remote_addr, Some(loopback(port)), "remote address");
            assert!(took >= LAN_PROBE_DEADLINE, "took {took:?}");
            assert!(took < Duration::from_secs(20), "took {took:?}");
            fx.then_server_accepted_and_data_flows(client).await;
        })
        .await;
    }

    #[tokio::test]
    async fn punch_still_wins_when_no_candidate_answers() {
        within(async {
            // given: the only candidate is silent, nothing vetoes the hole-punch
            let mut fx = setup().await;
            let lever = probing(&[Candidate::Silent], None);
            // when: the client connects
            let client = fx.when_connect(lever.opts.clone()).await;
            // then: the punch carries the connection
            let client = client.expect("connect by hole-punch");
            fx.then_server_accepted_and_data_flows(client).await;
        })
        .await;
    }

    #[tokio::test]
    async fn nothing_answers_and_connect_fails() {
        within(async {
            // given: the only candidate is silent and the hole-punch is vetoed
            let fx = setup().await;
            let lever = probing(&[Candidate::Silent], veto());
            // when: the client connects
            let started = Instant::now();
            let result = fx.when_connect(lever.opts.clone()).await;
            let took = started.elapsed();
            // then: an error after the candidate's deadline
            then_aborted(result);
            assert!(took >= LAN_PROBE_DEADLINE, "failed after {took:?}");
        })
        .await;
    }

    #[tokio::test]
    async fn without_local_connection_nothing_is_probed() {
        within(async {
            // given: no local connection; the lever offers a reachable candidate; punch vetoed
            let fx = setup().await;
            let lever = probing(&[Candidate::Advertised], veto());
            let opts = ConnectOpts {
                local_connection: false,
                ..lever.opts.clone()
            };
            // when: the client connects
            let result = fx.when_connect(opts).await;
            // then: the candidate was not used, the veto decides
            then_aborted(result);
        })
        .await;
    }
}

mod lan_probes {
    use super::*;

    #[tokio::test]
    async fn node_socket_answers_the_dht_ping() {
        within(async {
            // given: a cluster node as the candidate (it answers DHT pings, not the 1-byte probe)
            let cl = cluster(3).await;
            let client = node(&cl, true).await;
            let mut probes = probes_of(&client).await;
            // when: the candidate is reached
            let target = cl.addrs[0].clone();
            let hit = timeout(Duration::from_secs(10), probes.reach(&target)).await;
            // then: the answer is the ping's, without a prober socket
            let hit = hit.expect("no answer in 10 s");
            let (addr, socket) = hit.expect("the cluster node did not answer the ping");
            assert_eq!(addr, loopback(target.port), "answering address");
            assert!(
                socket.is_none(),
                "a ping answer must not carry a prober socket"
            );
        })
        .await;
    }

    #[tokio::test]
    async fn puncher_answers_the_probe_with_the_prober_socket() {
        within(async {
            // given: a passive puncher as the candidate
            let cl = cluster(3).await;
            let client = node(&cl, true).await;
            let (port, _puncher) = passive_puncher(&client).await;
            let mut probes = probes_of(&client).await;
            // when: the candidate is reached
            let hit = timeout(Duration::from_secs(30), probes.reach(&peer(port))).await;
            // then: the answer is the probe's, with the prober's socket
            let hit = hit.expect("no answer in 30 s");
            let (addr, socket) = hit.expect("the passive puncher did not answer the probe");
            assert_eq!(addr, loopback(port), "answering address");
            assert!(
                socket.is_some(),
                "a probe answer must carry the prober socket"
            );
        })
        .await;
    }
}

/// Where a probed candidate points on loopback: a socket nobody reads, or the advertised puncher port.
#[derive(Clone, Copy)]
enum Candidate {
    Silent,
    Advertised,
}

/// Connect options that replace the private candidates, and what the lever saw.
struct Probing {
    opts: ConnectOpts,
    /// The first port of the server's advertisement as the lever saw it (0 before any connect).
    advertised: Arc<AtomicU16>,
    _silent: std::net::UdpSocket,
}

/// A connect whose private candidates are `list`, with `holepunch` as the veto.
fn probing(list: &[Candidate], holepunch: Option<HolepunchFilter>) -> Probing {
    let silent = std::net::UdpSocket::bind("127.0.0.1:0").expect("bind");
    let silent_port = silent.local_addr().expect("addr").port();
    let advertised = Arc::new(AtomicU16::new(0));
    let (list, seen) = (list.to_vec(), advertised.clone());
    let rewrite = move |ads: &[Ipv4Peer]| -> Vec<Ipv4Peer> {
        let port = ads.first().map_or(0, |p| p.port);
        seen.store(port, Ordering::SeqCst);
        let pick = |c: &Candidate| match c {
            Candidate::Silent => peer(silent_port),
            Candidate::Advertised => peer(port),
        };
        list.iter().map(pick).collect()
    };
    let opts = ConnectOpts {
        holepunch,
        lan_rewrite: Some(LanRewrite(Arc::new(rewrite))),
        ..ConnectOpts::default()
    };
    Probing {
        opts,
        advertised,
        _silent: silent,
    }
}

/// Vetoes every hole-punch, so only the LAN path can carry the connection.
fn veto() -> Option<HolepunchFilter> {
    Some(HolepunchFilter::new(|_| false))
}

/// A cluster, a firewalled server (`key(1)`) and a firewalled client node, addresses settled.
struct Fixture {
    _cl: Cluster,
    server: Served,
    client: Node,
}

impl Fixture {
    /// The client (`key(2)`) connects to the server with `opts`.
    async fn when_connect(&self, opts: ConnectOpts) -> Result<PeerConnection, HyperDhtError> {
        let handle = &self.client.handle;
        handle
            .connect_with_options(&key(2), key(1).public_key, &[], &self.client.rt, opts)
            .await
    }

    /// The server accepts the client's key (30 s at most) and the echo over the pair is intact.
    async fn then_server_accepted_and_data_flows(&mut self, client: PeerConnection) {
        let next = timeout(Duration::from_secs(30), self.server.incoming.recv()).await;
        let server = next
            .expect("no incoming connection in 30 s")
            .expect("incoming channel closed");
        assert_eq!(server.remote_public_key, key(2).public_key, "server side");
        let bytes: Vec<u8> = (0..20_000usize).map(|i| (i % 251) as u8).collect();
        let expected = sha256(&bytes);
        let server_end = tokio::spawn(echo(server));
        let got = roundtrip(client, bytes).await;
        let _ = server_end.await;
        assert_eq!(got, expected, "echo came back different");
    }
}

async fn setup() -> Fixture {
    let cl = cluster(3).await;
    let server = serve(node(&cl, true).await, &key(1), ServerOpts::default()).await;
    let client = node(&cl, true).await;
    for handle in [&server.handle, &client.handle] {
        let seen = settled_remote_address(handle).await;
        assert_eq!(seen.host, "127.0.0.1", "the shared address");
    }
    Fixture {
        _cl: cl,
        server,
        client,
    }
}

/// The connect failed with the veto's error, not with a dead stream.
fn then_aborted(result: Result<PeerConnection, HyperDhtError>) {
    let err = result.expect_err("connect returned Ok");
    assert!(matches!(err, HyperDhtError::HolepunchAborted), "{err:?}");
}

async fn probes_of(client: &Node) -> LanProbes<'_> {
    let probes = LanProbes::new(&client.handle, &client.rt, FIREWALL_UNKNOWN).await;
    probes.expect("probes")
}

/// A passive puncher on `0.0.0.0` (it reflects 1-byte probes): its port, and the puncher with its
/// events, to keep alive.
async fn passive_puncher(
    client: &Node,
) -> (u16, (Holepuncher, mpsc::UnboundedReceiver<HolepunchEvent>)) {
    let pool = SocketPool::new("0.0.0.0".into());
    let (tx, rx) = mpsc::unbounded_channel();
    let puncher = Holepuncher::new(&pool, &client.rt, true, false, FIREWALL_UNKNOWN, tx).await;
    let puncher = puncher.expect("passive puncher");
    let primary = puncher.primary_socket().expect("primary socket");
    let port = primary
        .socket
        .local_addr()
        .await
        .expect("local_addr")
        .port();
    (port, (puncher, rx))
}

fn peer(port: u16) -> Ipv4Peer {
    Ipv4Peer {
        host: "127.0.0.1".into(),
        port,
    }
}

fn loopback(port: u16) -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], port))
}
