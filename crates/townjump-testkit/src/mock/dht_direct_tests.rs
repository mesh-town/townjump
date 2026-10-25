// Tests of `MockDht` beside the direct-connect and address additions: the trait defaults and connections without an address.
use super::*;
use crate::conformance::key_pair;
use crate::mock::MockFactory;
use crate::mock::test_util::{node, serve};
use std::net::Ipv4Addr;
use std::time::Duration;
use townjump_dht::DhtFactory;

const SERVER_TAG: u8 = 0x74;
const WAIT: Duration = Duration::from_secs(2);

mod mock_dht {
    use super::*;

    #[tokio::test]
    async fn connect_direct_is_refused() {
        let dht = MockFactory::new().create(None).await.unwrap();
        let addr = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 1);

        let answer = tokio::time::timeout(WAIT, dht.connect_direct([1; 32], addr))
            .await
            .expect("the default answers at once");

        assert!(
            matches!(answer, Err(DhtError::Backend(_))),
            "got {answer:?}"
        );
    }

    #[tokio::test]
    async fn local_port_is_refused() {
        let dht = MockFactory::new().create(None).await.unwrap();

        let answer = tokio::time::timeout(WAIT, dht.local_port())
            .await
            .expect("the default answers at once");

        assert!(
            matches!(answer, Err(DhtError::Backend(_))),
            "got {answer:?}"
        );
    }

    #[tokio::test]
    async fn connection_has_no_remote_addr() {
        let factory = MockFactory::new();
        let (owner, client) = (node(&factory, 1).await, node(&factory, 2).await);
        let (_server, mut incoming) = serve(&owner, SERVER_TAG).await;

        let outgoing = client
            .connect(key_pair(SERVER_TAG).public_key, ConnectOpts::default())
            .await
            .unwrap();
        let accepted = incoming.accept().await.expect("the server got the call");

        assert_eq!(outgoing.remote_addr(), None, "outgoing half");
        assert_eq!(outgoing.rtt(), None, "outgoing half");
        assert_eq!(accepted.remote_addr(), None, "incoming half");
        assert_eq!(accepted.rtt(), None, "incoming half");
    }
}
