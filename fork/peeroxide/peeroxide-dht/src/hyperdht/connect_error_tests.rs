use super::{connect_failure, keep_terminal};
use crate::hyperdht::HyperDhtError;
use crate::rpc::DhtError;

fn swallowed() -> Vec<HyperDhtError> {
    vec![
        HyperDhtError::NoRelayNodes,
        HyperDhtError::HandshakeFailed("empty reply".into()),
        HyperDhtError::Dht(DhtError::RequestFailed("timeout".into())),
    ]
}

fn kept() -> Vec<HyperDhtError> {
    vec![
        HyperDhtError::FirewallRejected,
        HyperDhtError::HolepunchFailed,
        HyperDhtError::HolepunchAborted,
        HyperDhtError::StreamEstablishment("x".into()),
        HyperDhtError::Destroyed,
        HyperDhtError::Dht(DhtError::Destroyed),
    ]
}

#[test]
fn no_holders_and_only_swallowed_failures_is_peer_not_found() {
    for e in swallowed() {
        assert!(matches!(connect_failure(0, e), HyperDhtError::PeerNotFound));
    }
}

#[test]
fn holders_tried_and_only_swallowed_failures_is_no_relay_nodes() {
    for e in swallowed() {
        assert!(matches!(connect_failure(2, e), HyperDhtError::NoRelayNodes));
    }
}

#[test]
fn a_kept_failure_is_returned_whatever_the_holders() {
    for holders in [0, 3] {
        let e = connect_failure(holders, HyperDhtError::HolepunchFailed);
        assert!(matches!(e, HyperDhtError::HolepunchFailed), "{e:?}");
    }
    for e in kept() {
        let text = e.to_string();
        assert_eq!(connect_failure(1, e).to_string(), text);
    }
}

#[test]
fn a_swallowed_failure_does_not_replace_a_kept_one() {
    for new in swallowed() {
        let e = keep_terminal(HyperDhtError::HolepunchAborted, new);
        assert!(matches!(e, HyperDhtError::HolepunchAborted), "{e:?}");
    }
    let e = keep_terminal(HyperDhtError::NoRelayNodes, HyperDhtError::FirewallRejected);
    assert!(matches!(e, HyperDhtError::FirewallRejected));
    let e = keep_terminal(
        HyperDhtError::HolepunchAborted,
        HyperDhtError::HolepunchFailed,
    );
    assert!(matches!(e, HyperDhtError::HolepunchFailed));
}
