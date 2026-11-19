// Tests for the `error` module: Display of every ServerError variant, and `NotStarted` from `put`/`get`;
// `key`/`info` before `start`. Display strings are the JS exception `.message` texts.
use super::*;
use crate::HolesailServer;
use townjump_dht::DhtError;
use townjump_keys::KeysError;
use townjump_logger::{Fatal, Logger};
use townjump_testkit::capture::Capture;
use townjump_testkit::mock::{MockDht, MockFactory};

async fn unstarted_server() -> HolesailServer<MockDht> {
    let factory = MockFactory::default();
    let logger = Logger::new(Some(0.0), Capture::new().shared(), Capture::new().shared());
    let (fatal, _rx) = Fatal::channel();
    HolesailServer::new(&factory, logger, fatal)
        .await
        .expect("new failed")
}

mod display {
    use super::*;

    #[test]
    fn not_started_text() {
        assert_eq!(ServerError::NotStarted.to_string(), "server is not started");
    }

    #[test]
    fn dht_variant_is_transparent() {
        let cases = [
            (DhtError::Destroyed, "NODE_DESTROYED: Node destroyed"),
            (DhtError::PeerNotFound, "PEER_NOT_FOUND: Peer not found"),
            (
                DhtError::ConnectionFailed("Could not connect to peer".to_string()),
                "PEER_CONNECTION_FAILED: Could not connect to peer",
            ),
            (
                DhtError::HolepunchAborted,
                "HOLEPUNCH_ABORTED: Holepunch aborted",
            ),
            (
                DhtError::Backend("Too few nodes responded".to_string()),
                "Too few nodes responded",
            ),
        ];
        for (error, text) in cases {
            assert_eq!(ServerError::from(error).to_string(), text);
        }
    }

    #[test]
    fn dht_io_error_shows_the_io_message() {
        let io = std::io::Error::new(std::io::ErrorKind::AddrInUse, "address in use");
        assert_eq!(
            ServerError::from(DhtError::from(io)).to_string(),
            "address in use"
        );
    }

    #[test]
    fn keys_variant_is_transparent() {
        let seed = ServerError::from(KeysError::SeedBytes);
        assert_eq!(
            seed.to_string(),
            "seed must be 'crypto_sign_SEEDBYTES' bytes"
        );
        let bad = ServerError::from(KeysError::InvalidCharacter {
            ch: "l".to_string(),
            position: 11,
        });
        assert_eq!(
            bad.to_string(),
            "Invalid character in base32 input: \"l\" at position 11"
        );
    }
}

mod before_start {
    use super::*;

    #[tokio::test]
    async fn put_is_not_started() {
        let server = unstarted_server().await;
        let result = server.put("data").await;
        assert!(matches!(result, Err(ServerError::NotStarted)), "{result:?}");
    }

    #[tokio::test]
    async fn get_is_not_started() {
        let server = unstarted_server().await;
        let result = server.get().await;
        assert!(matches!(result, Err(ServerError::NotStarted)), "{result:?}");
    }

    #[tokio::test]
    async fn key_is_empty() {
        assert_eq!(unstarted_server().await.key(), "");
    }

    #[tokio::test]
    async fn info_is_the_unstarted_snapshot() {
        let info = unstarted_server().await.info();
        assert_eq!(info.state, None);
        assert_eq!(info.key, "");
        assert_eq!(info.public_key, "");
        assert_eq!(info.seed, None);
        assert!(!info.secure);
    }
}
