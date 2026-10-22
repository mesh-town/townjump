// Tests of the default `Dht` methods: a backend that knows nothing about modes.
use super::*;
use crate::{ConnectOpts, DhtConnection, Incoming, MutableRecord, NodeMode, ServerOpts};
use townjump_keys::KeyPair;

struct Plain;
#[derive(Clone)]
struct PlainServer;

impl DhtServer for PlainServer {
    async fn listen(&self, _kp: KeyPair) -> Result<(), DhtError> {
        Ok(())
    }
    async fn close(&self) -> Result<(), DhtError> {
        Ok(())
    }
}

impl Dht for Plain {
    type Server = PlainServer;
    fn create_server(&self, _opts: ServerOpts) -> Result<(PlainServer, Incoming), DhtError> {
        Err(DhtError::Destroyed)
    }
    async fn connect(&self, _r: [u8; 32], _o: ConnectOpts) -> Result<DhtConnection, DhtError> {
        Err(DhtError::Destroyed)
    }
    async fn mutable_put(&self, _k: &KeyPair, _v: &[u8], _s: Option<u64>) -> Result<u64, DhtError> {
        Ok(0)
    }
    async fn mutable_get(
        &self,
        _p: [u8; 32],
        _l: Option<bool>,
    ) -> Result<Option<MutableRecord>, DhtError> {
        Ok(None)
    }
    async fn destroy(&self) -> Result<(), DhtError> {
        Ok(())
    }
    async fn suspend(&self) -> Result<(), DhtError> {
        Ok(())
    }
    async fn resume(&self) -> Result<(), DhtError> {
        Ok(())
    }
}

mod defaults {
    use super::*;
    use std::time::Duration;

    #[test]
    fn node_mode_is_the_default_mode() {
        assert_eq!(Plain.node_mode(), NodeMode::default());
    }

    #[tokio::test]
    async fn set_adaptive_is_a_backend_error() {
        let err = Plain.set_adaptive(true).await.expect_err("must be refused");
        assert!(matches!(err, DhtError::Backend(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn mode_changed_never_resolves() {
        let waited = tokio::time::timeout(
            Duration::from_millis(100),
            Plain.mode_changed(NodeMode::default()),
        )
        .await;
        assert!(waited.is_err(), "mode_changed resolved: {waited:?}");
    }
}

struct PlainFactory;

impl DhtFactory for PlainFactory {
    type Dht = Plain;
    async fn create(&self, _identity: Option<KeyPair>) -> Result<Plain, DhtError> {
        Ok(Plain)
    }
}

mod nodes_defaults {
    use super::*;

    #[tokio::test]
    async fn plain_to_array_is_empty() {
        assert!(Plain.to_array(None).await.expect("export").is_empty());
    }

    #[tokio::test]
    async fn plain_set_nodes_is_ignored() {
        let mut factory = PlainFactory;
        factory.set_nodes(vec!["1.2.3.4:5".parse().unwrap()]);
        assert!(factory.create(None).await.is_ok());
    }
}
