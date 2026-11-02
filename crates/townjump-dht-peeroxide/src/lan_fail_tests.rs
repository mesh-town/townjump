// Tests of the fast refusal: `connect` does not wait out the LAN window once the DHT path got a final answer, and a
// destroyed node sends no probe. A child of lan_race_tests: shares its helpers.
use super::added::sniffer;
use super::*;
use crate::error::refused;
use crate::lan_race::{DhtFailure, lan_path, race};
use peeroxide_dht::hyperdht::{HyperDhtError, PeerConnection};
use peeroxide_dht::rpc::DhtError as RpcError;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connect_fails_at_once_when_the_node_is_destroyed_meanwhile() {
    let _lan = LAN.lock().await;
    let client = node(&lan_closed_factory(), setup(group_port(), ProbeStart::Now)).await;
    let connecting = async {
        let result = client.connect([9; 32], Default::default()).await;
        (result, Instant::now())
    };
    let destroying = async {
        tokio::time::sleep(Duration::from_millis(300)).await;
        let at = Instant::now();
        let _ = client.destroy().await;
        at
    };
    let ((result, ended), destroyed_at) = within(WAIT, "connect", async {
        tokio::join!(connecting, destroying)
    })
    .await;
    assert!(
        matches!(result, Err(DhtError::Destroyed)),
        "got {:?}",
        result.err()
    );
    let late = ended.saturating_duration_since(destroyed_at);
    assert!(late < Duration::from_secs(1), "connect lingered {late:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lan_path_after_destroy_sends_no_probe() {
    let _lan = LAN.lock().await;
    let port = group_port();
    let sniffer = sniffer(port);
    let dht = node(&lan_closed_factory(), setup(port, ProbeStart::Now)).await;
    dht.destroy().await.expect("destroy");
    let found = lan_path(&dht.node, [8; 32], &Notify::new()).await;
    assert!(found.is_none());
    let mut buf = [0u8; 64];
    assert!(
        sniffer.recv(&mut buf).is_err(),
        "a probe was sent after destroy"
    );
}

#[tokio::test(start_paused = true)]
async fn race_returns_a_refusal_without_waiting_for_the_lan_window() {
    let dht = async {
        tokio::time::sleep(Duration::from_millis(100)).await;
        Err::<PeerConnection, _>(DhtFailure {
            error: DhtError::Destroyed,
            refused: true,
        })
    };
    let lan = async {
        tokio::time::sleep(Duration::from_secs(3)).await;
        None::<PeerConnection>
    };
    let begun = tokio::time::Instant::now();
    let result = race(dht, lan).await;
    assert!(matches!(result, Err(DhtError::Destroyed)));
    assert_eq!(begun.elapsed(), Duration::from_millis(100));
}

#[test]
fn refused_marks_only_what_the_lan_cannot_fix() {
    let yes = [
        HyperDhtError::Destroyed,
        HyperDhtError::ChannelClosed,
        HyperDhtError::FirewallRejected,
        HyperDhtError::Dht(RpcError::Destroyed),
        HyperDhtError::Dht(RpcError::ChannelClosed),
        HyperDhtError::Dht(RpcError::Suspended),
    ];
    let no = [
        HyperDhtError::PeerNotFound,
        HyperDhtError::NoRelayNodes,
        HyperDhtError::HandshakeFailed("x".into()),
        HyperDhtError::HolepunchFailed,
        HyperDhtError::HolepunchAborted,
        HyperDhtError::Dht(RpcError::BootstrapFailed),
    ];
    assert!(yes.iter().all(refused), "a refusal is not marked");
    assert!(!no.iter().any(refused), "an open failure is marked");
}
