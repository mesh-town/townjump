// Mock test helpers: a node with a key by tag, a server listening on the tag key, the "nothing arrived" check.
use crate::conformance::key_pair;
use crate::mock::{MockDht, MockFactory, MockServer};
use std::time::Duration;
use townjump_dht::{Dht, DhtFactory, DhtServer, Incoming, ServerOpts};

const SILENCE: Duration = Duration::from_millis(100);

pub(crate) async fn node(factory: &MockFactory, tag: u8) -> MockDht {
    factory.create(Some(key_pair(tag))).await.unwrap()
}

/// Server of a node without a firewall; listens on the public key `key_pair(tag)`.
pub(crate) async fn serve(dht: &MockDht, tag: u8) -> (MockServer, Incoming) {
    serve_with(dht, tag, ServerOpts::default()).await
}

pub(crate) async fn serve_with(dht: &MockDht, tag: u8, opts: ServerOpts) -> (MockServer, Incoming) {
    let (server, incoming) = dht.create_server(opts).unwrap();
    server.listen(key_pair(tag)).await.unwrap();
    (server, incoming)
}

/// Nothing reached the incoming queue within 100 ms.
pub(crate) async fn nothing_arrives(incoming: &mut Incoming) -> bool {
    tokio::time::timeout(SILENCE, incoming.accept())
        .await
        .is_err()
}
