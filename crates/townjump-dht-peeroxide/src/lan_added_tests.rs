// Added acceptance tests of the LAN part: a live DHT that lacks the key, the pure `race` (virtual time), the responder
// life after `destroy`, the switch-off, the end of bootstrap. A child of lan_race_tests: shares its helpers.
use super::*;
use crate::lan_race::{DhtFailure, race};
use peeroxide_dht::hyperdht::PeerConnection;

/// A socket on the group port that has joined the group on every interface: hears the probes.
pub(super) fn sniffer(port: u16) -> UdpSocket {
    let sniffer = UdpSocket::bind(("0.0.0.0", port)).expect("sniffer");
    sniffer
        .set_read_timeout(Some(Duration::from_millis(200)))
        .expect("timeout");
    for iface in if_addrs::get_if_addrs().expect("interfaces") {
        if let std::net::IpAddr::V4(ip) = iface.ip() {
            let _ = sniffer.join_multicast_v4(&LanConfig::default().group, &ip);
        }
    }
    sniffer
}

fn target(key: &KeyPair) -> [u8; 32] {
    peeroxide_dht::crypto::hash(&key.public_key)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fails_with_peer_not_found_when_the_live_dht_does_not_know_the_key() {
    let _lan = LAN.lock().await;
    let net = support::start_testnet();
    let port = group_port();
    let sniffer = sniffer(port);
    let client = node(&net.factory, setup(port, ProbeStart::Quiet)).await;
    let failed = within(WAIT, "connect", client.connect([9; 32], Default::default())).await;
    assert!(
        matches!(failed, Err(DhtError::PeerNotFound)),
        "got {:?}",
        failed.err()
    );
    let mut buf = [0u8; 64];
    assert!(
        sniffer.recv(&mut buf).is_ok(),
        "no LAN probe after the DHT path failed"
    );
    let _ = client.destroy().await;
}

#[tokio::test(start_paused = true)]
async fn race_still_waits_for_the_lan_after_an_open_failure() {
    let dht = async {
        tokio::time::sleep(Duration::from_millis(100)).await;
        Err::<PeerConnection, DhtFailure>(DhtError::Backend("a".into()).into())
    };
    let lan = async {
        tokio::time::sleep(Duration::from_secs(3)).await;
        None::<PeerConnection>
    };
    let begun = tokio::time::Instant::now();
    let result = race(dht, lan).await;
    assert!(matches!(&result, Err(DhtError::Backend(m)) if m == "a"));
    assert_eq!(begun.elapsed(), Duration::from_secs(3));
}

#[tokio::test(start_paused = true)]
async fn race_returns_the_dht_error_when_the_lan_gives_up_first() {
    let dht = async {
        tokio::time::sleep(Duration::from_secs(2)).await;
        Err::<PeerConnection, DhtFailure>(DhtError::Backend("b".into()).into())
    };
    let lan = async {
        tokio::time::sleep(Duration::from_millis(100)).await;
        None::<PeerConnection>
    };
    let begun = tokio::time::Instant::now();
    let result = race(dht, lan).await;
    assert!(matches!(&result, Err(DhtError::Backend(m)) if m == "b"));
    assert_eq!(begun.elapsed(), Duration::from_secs(2));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn destroyed_node_ignores_late_lan_registrations() {
    let _lan = LAN.lock().await;
    let port = group_port();
    let (dht, _server, _incoming, _key) = serving(port, &closed_factory()).await;
    let (k2, k3) = (key_pair(SEED + 1), key_pair(SEED + 2));
    dht.node.lan.register(target(&k2), 4242);
    assert_eq!(ask_some(port, &k2).await.map(|a| a.port()), Some(4242));
    dht.destroy().await.expect("destroy");
    assert_eq!(ask(port, &k2).await, None, "the old registration answers");
    dht.node.lan.register(target(&k3), 4243);
    assert_eq!(ask(port, &k3).await, None, "a late registration answers");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn discovery_off_starts_no_responder() {
    let _lan = LAN.lock().await;
    let port = group_port();
    let (control, _server, _incoming, key) = serving(port, &closed_factory()).await;
    let off = PeeroxideFactory {
        lan_discovery: false,
        ..closed_factory()
    };
    let quiet = node(&off, setup(port, ProbeStart::Now)).await;
    let (srv, _inc) = quiet.create_server(ServerOpts::default()).expect("server");
    let silent = key_pair(SEED + 1);
    within(WAIT, "listen", srv.listen(silent.clone()))
        .await
        .expect("listen");
    assert!(ask_some(port, &key).await.is_some(), "the control is deaf");
    assert_eq!(
        ask(port, &silent).await,
        None,
        "discovery off, yet answered"
    );
    let _ = tokio::join!(quiet.destroy(), control.destroy());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bootstrap_done_is_false_while_the_bootstrap_runs() {
    let dht = node(&closed_factory(), setup(group_port(), ProbeStart::Quiet)).await;
    assert!(!dht.node.bootstrap_done().await, "done before it began");
    within(WAIT, "ready", dht.node.ready())
        .await
        .expect("ready");
    assert!(dht.node.bootstrap_done().await, "not done after ready");
    let _ = dht.destroy().await;
}
