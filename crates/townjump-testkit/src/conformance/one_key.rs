// JS: hyperdht/lib/hyperdht.js `connect` with one keyPair and `reusableSocket`: five connections of one identity at once.
// Related: setup.rs (nodes), data.rs (writing and reading streams). Keys: 0x31 server, 0x32 client.
use super::data::{read_all, send_and_close};
use super::keys::key_pair;
use super::setup::{listen_on, node};
use super::wait::{accept_one, connect_ok};
use std::collections::BTreeSet;
use std::sync::Arc;
use tokio::task::JoinSet;
use townjump_dht::{Dht, DhtConnection, DhtFactory};

const CONNECTS: u8 = 5;

/// Client `i` writes byte `i` and closes its write side; the connection stays with the client until the scenario ends.
async fn dial<D: Dht>(client: Arc<D>, server_pk: [u8; 32], i: u8) -> DhtConnection {
    let mut conn = connect_ok(&*client, server_pk, "one_key: connect").await;
    send_and_close(&mut conn.stream, &[i]).await;
    conn
}

/// Five `connect` calls at once with one identity: the server sees five incoming connections with the client identity, bytes {0..4}.
pub async fn one_key_five_connects<F: DhtFactory>(factory: &F) {
    let mut server = listen_on(factory, key_pair(0x31), None).await;
    let client = Arc::new(node(factory, 0x32).await);
    let client_pk = key_pair(0x32).public_key;
    let mut dials = JoinSet::new();
    for i in 0..CONNECTS {
        dials.spawn(dial(client.clone(), server.pk, i));
    }
    let mut seen = BTreeSet::new();
    for _ in 0..CONNECTS {
        let mut conn = accept_one(&mut server.incoming, "one_key: accept").await;
        assert_eq!(
            conn.remote_public_key, client_pk,
            "one_key: client identity"
        );
        seen.extend(read_all(&mut conn.stream, "one_key: read").await);
    }
    assert_eq!(
        seen,
        (0..CONNECTS).collect::<BTreeSet<u8>>(),
        "one_key: client bytes"
    );
    let mut held = Vec::new();
    while let Some(done) = dials.join_next().await {
        held.push(done.expect("one_key: client task"));
    }
    assert_eq!(
        held.len(),
        usize::from(CONNECTS),
        "one_key: client connections"
    );
}
