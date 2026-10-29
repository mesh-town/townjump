// JS: hyperdht/lib/hyperdht.js: `new HyperDHT({ keyPair })`, `createServer({ firewall })`, `server.listen(keyPair)`.
// Related: keys.rs (pairs by tag), wait.rs (waits). Nodes and servers are created with `reusable_socket`.
use super::keys::key_pair;
use super::wait::{accept_one, connect_ok};
use townjump_dht::{
    Dht, DhtConnection, DhtFactory, DhtServer, Firewall, Incoming, KeyPair, ServerOpts,
};

/// Node listening on key `pk`, with the queue of its incoming connections.
pub struct Listening<D: Dht> {
    pub dht: D,
    pub server: D::Server,
    pub incoming: Incoming,
    pub pk: [u8; 32],
}

/// Client node with a pair by tag.
pub async fn node<F: DhtFactory>(factory: &F, tag: u8) -> F::Dht {
    factory
        .create(Some(key_pair(tag)))
        .await
        .expect("create: failed to create a node")
}

/// Node with pair `kp`, a server with `firewall` (None - let everyone in), `listen` on `kp`.
pub async fn listen_on<F: DhtFactory>(
    factory: &F,
    kp: KeyPair,
    firewall: Option<Firewall>,
) -> Listening<F::Dht> {
    let dht = factory
        .create(Some(kp.clone()))
        .await
        .expect("create: server");
    let opts = ServerOpts {
        firewall,
        reusable_socket: true,
        ..Default::default()
    };
    let (server, incoming) = dht.create_server(opts).expect("create_server");
    server.listen(kp.clone()).await.expect("listen");
    Listening {
        dht,
        server,
        incoming,
        pk: kp.public_key,
    }
}

/// The client connects to the server, the server accepts; identities on both sides are checked.
/// Returns `(client connection, server connection)`.
pub async fn connect_pair<D: Dht>(
    client: &D,
    client_pk: [u8; 32],
    server: &mut Listening<D>,
    what: &str,
) -> (DhtConnection, DhtConnection) {
    let dial = connect_ok(client, server.pk, what);
    let (dialed, accepted) = tokio::join!(dial, accept_one(&mut server.incoming, what));
    assert_eq!(
        dialed.remote_public_key, server.pk,
        "{what}: client does not see the server key"
    );
    assert_eq!(
        accepted.remote_public_key, client_pk,
        "{what}: server does not see the client identity"
    );
    (dialed, accepted)
}
