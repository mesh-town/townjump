// Tests of the `error` module: `Display` of each variant = `.message` of DHTError from hyperdht/lib/errors.js (`<CODE>: <text>`).
// Strings obtained by running JS (`require('hyperdht/lib/errors')`, node 24.4); `Io` and `Backend` have no code.
use super::*;
use std::io;

mod dht_codes {
    use super::*;

    #[test]
    fn destroyed_reads_node_destroyed() {
        assert_eq!(
            DhtError::Destroyed.to_string(),
            "NODE_DESTROYED: Node destroyed"
        );
    }

    #[test]
    fn peer_not_found_reads_peer_not_found() {
        assert_eq!(
            DhtError::PeerNotFound.to_string(),
            "PEER_NOT_FOUND: Peer not found"
        );
    }

    #[test]
    fn connection_failed_with_the_default_text_reads_could_not_connect_to_peer() {
        let err = DhtError::ConnectionFailed("Could not connect to peer".into());

        assert_eq!(
            err.to_string(),
            "PEER_CONNECTION_FAILED: Could not connect to peer"
        );
    }

    #[test]
    fn connection_failed_puts_the_given_text_after_the_code() {
        let err = DhtError::ConnectionFailed("boom".into());

        assert_eq!(err.to_string(), "PEER_CONNECTION_FAILED: boom");
    }

    #[test]
    fn holepunch_aborted_reads_holepunch_aborted() {
        assert_eq!(
            DhtError::HolepunchAborted.to_string(),
            "HOLEPUNCH_ABORTED: Holepunch aborted"
        );
    }
}

mod io_error {
    use super::*;

    #[test]
    fn display_is_the_io_error_text_without_any_prefix() {
        let io_err = io::Error::new(
            io::ErrorKind::ConnectionRefused,
            "connect ECONNREFUSED 127.0.0.1:3000",
        );

        let err = DhtError::from(io_err);

        assert_eq!(err.to_string(), "connect ECONNREFUSED 127.0.0.1:3000");
    }

    #[test]
    fn question_mark_wraps_the_io_error_and_keeps_its_kind() {
        let err = fails_with_io(io::ErrorKind::TimedOut).unwrap_err();

        assert!(
            matches!(&err, DhtError::Io(inner) if inner.kind() == io::ErrorKind::TimedOut),
            "{err:?}"
        );
    }
}

mod backend {
    use super::*;

    #[test]
    fn text_is_shown_as_is_without_a_code() {
        let err = DhtError::Backend("bootstrap: bad address".into());

        assert_eq!(err.to_string(), "bootstrap: bad address");
    }
}

mod bounds {
    use super::*;

    #[tokio::test]
    async fn error_crosses_task_boundary_and_boxes_into_dyn_error() {
        let joined = tokio::spawn(async { Err::<(), DhtError>(DhtError::Destroyed) }).await;

        let boxed: Box<dyn std::error::Error + Send + Sync> = joined.unwrap().unwrap_err().into();

        assert!(matches!(
            boxed.downcast_ref::<DhtError>(),
            Some(DhtError::Destroyed)
        ));
    }
}

fn fails_with_io(kind: io::ErrorKind) -> Result<(), DhtError> {
    let result: io::Result<()> = Err(io::Error::from(kind));
    result?;
    Ok(())
}
