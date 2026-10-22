// Tests of the `error` module: `map_error` — every `HyperDhtError` → `DhtError` branch, texts as in JS `DHTError`.
// Pure functions, no network; texts and codes — hyperdht/lib/errors.js, outcome of a firewall rejection — firewall.md.
use super::*;
use libudx::UdxError;
use peeroxide_dht::rpc::DhtError as RpcError;
use std::io;

mod node_destroyed {
    use super::*;

    #[test]
    fn maps_to_destroyed() {
        let mapped = map_error(HyperDhtError::Destroyed);
        assert!(matches!(mapped, DhtError::Destroyed), "got {mapped:?}");
    }

    #[test]
    fn has_the_js_text() {
        assert_eq!(
            text(HyperDhtError::Destroyed),
            "NODE_DESTROYED: Node destroyed"
        );
    }

    #[test]
    fn destroyed_in_the_rpc_layer_maps_the_same_way() {
        let mapped = map_error(HyperDhtError::Dht(RpcError::Destroyed));
        assert!(matches!(mapped, DhtError::Destroyed), "got {mapped:?}");
    }
}

mod peer_not_found {
    use super::*;

    #[test]
    fn maps_to_peer_not_found() {
        let mapped = map_error(HyperDhtError::PeerNotFound);
        assert!(matches!(mapped, DhtError::PeerNotFound), "got {mapped:?}");
    }

    #[test]
    fn has_the_js_text() {
        assert_eq!(
            text(HyperDhtError::PeerNotFound),
            "PEER_NOT_FOUND: Peer not found"
        );
    }
}

mod holepunch_aborted {
    use super::*;

    #[test]
    fn maps_to_holepunch_aborted() {
        let mapped = map_error(HyperDhtError::HolepunchAborted);
        assert!(
            matches!(mapped, DhtError::HolepunchAborted),
            "got {mapped:?}"
        );
    }

    #[test]
    fn has_the_js_text() {
        assert_eq!(
            text(HyperDhtError::HolepunchAborted),
            "HOLEPUNCH_ABORTED: Holepunch aborted"
        );
    }
}

mod suspended_node {
    use super::*;

    // JS: an outgoing action of a suspended node — `SUSPENDED: Suspended` (hyperdht/lib/errors.js).
    #[test]
    fn rpc_suspended_and_the_direct_constructor_give_the_js_text() {
        let via_rpc = map_error(HyperDhtError::Dht(RpcError::Suspended));
        let expected = DhtError::Backend("SUSPENDED: Suspended".to_string());
        assert_eq!(via_rpc.to_string(), expected.to_string());
        assert!(
            matches!(&via_rpc, DhtError::Backend(t) if t == "SUSPENDED: Suspended"),
            "got {via_rpc:?}"
        );
        let direct = suspended();
        assert!(
            matches!(&direct, DhtError::Backend(t) if t == "SUSPENDED: Suspended"),
            "got {direct:?}"
        );
    }
}

mod refused_by_the_remote_firewall {
    use super::*;

    // For a JS client a firewall rejection is indistinguishable from "no server": the outcome is PEER_CONNECTION_FAILED.
    #[test]
    fn is_a_connection_failure() {
        let mapped = map_error(HyperDhtError::FirewallRejected);
        assert!(
            matches!(mapped, DhtError::ConnectionFailed(_)),
            "got {mapped:?}"
        );
    }

    #[test]
    fn carries_the_peer_connection_failed_code() {
        let shown = text(HyperDhtError::FirewallRejected);
        assert!(
            shown.starts_with("PEER_CONNECTION_FAILED: "),
            "text without the PEER_CONNECTION_FAILED code: {shown:?}"
        );
    }
}

mod socket_error {
    use super::*;

    // JS: a socket error is a plain `Error` with `code` (ECONNRESET), not `DHTError`; the kind is kept, hence `Io` (client.md, «Fork error map (`map_error` in `townjump-dht-peeroxide`)»).
    #[test]
    fn keeps_the_io_kind() {
        let reset = io::Error::from(io::ErrorKind::ConnectionReset);
        let mapped = map_error(HyperDhtError::Udx(UdxError::Io(reset)));
        assert!(
            matches!(&mapped, DhtError::Io(e) if e.kind() == io::ErrorKind::ConnectionReset),
            "got {mapped:?}"
        );
    }
}

mod failures_without_a_dedicated_variant {
    use super::*;

    // The map is settled in `client.md`, «Fork error map (`map_error` in `townjump-dht-peeroxide`)»: the code comes from `hyperdht/lib/errors.js`, the fork text is kept after the code.
    #[test]
    fn handshake_failed_is_a_bad_handshake_reply() {
        let shown = text(HyperDhtError::HandshakeFailed("bad reply".into()));
        assert_eq!(shown, "BAD_HANDSHAKE_REPLY: bad reply");
    }

    #[test]
    fn stream_establishment_is_a_connection_failure() {
        let shown = text(HyperDhtError::StreamEstablishment("udx refused".into()));
        assert_eq!(shown, "PEER_CONNECTION_FAILED: udx refused");
    }

    #[test]
    fn holepunch_failed_is_cannot_holepunch() {
        let shown = text(HyperDhtError::HolepunchFailed);
        assert_eq!(shown, "CANNOT_HOLEPUNCH: Cannot holepunch to remote");
    }

    #[test]
    fn relay_aborted_keeps_the_js_code() {
        let shown = text(HyperDhtError::RelayAborted);
        assert_eq!(shown, "RELAY_ABORTED: Relay aborted");
    }

    #[test]
    fn no_relay_nodes_is_a_connection_failure_with_the_js_default_text() {
        let shown = text(HyperDhtError::NoRelayNodes);
        assert_eq!(shown, "PEER_CONNECTION_FAILED: Could not connect to peer");
    }
}

mod every_variant {
    use super::*;

    #[test]
    fn gives_a_non_empty_text() {
        for error in samples() {
            let label = format!("{error:?}");
            let shown = map_error(error).to_string();
            assert!(!shown.trim().is_empty(), "{label}: empty error text");
        }
    }
}

/// Error text as the user will see it.
fn text(error: HyperDhtError) -> String {
    map_error(error).to_string()
}

/// Fork errors that can be built without a network.
fn samples() -> Vec<HyperDhtError> {
    vec![
        HyperDhtError::Destroyed,
        HyperDhtError::InvalidSignature,
        HyperDhtError::InvalidHash,
        HyperDhtError::ChannelClosed,
        HyperDhtError::PeerNotFound,
        HyperDhtError::NoRelayNodes,
        HyperDhtError::HandshakeFailed("x".into()),
        HyperDhtError::HolepunchFailed,
        HyperDhtError::HolepunchAborted,
        HyperDhtError::RelayAborted,
        HyperDhtError::FirewallRejected,
        HyperDhtError::StreamEstablishment("x".into()),
        HyperDhtError::Dht(RpcError::ChannelClosed),
        HyperDhtError::Dht(RpcError::BootstrapFailed),
        HyperDhtError::Dht(RpcError::RequestFailed("x".into())),
        HyperDhtError::Udx(UdxError::StreamClosed),
        HyperDhtError::Udx(UdxError::RuntimeGone),
    ]
}
