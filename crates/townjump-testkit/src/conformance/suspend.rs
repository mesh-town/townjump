// JS: hyperdht/lib/hyperdht.js `suspend`/`resume` and `suspended` in server.js: a suspended node does not accept incoming connections.
// Related: setup.rs, wait.rs. Keys: 0x61 server, 0x62 client.
use super::keys::key_pair;
use super::setup::{connect_pair, listen_on, node};
use super::wait::{assert_empty, connect_refused};
use townjump_dht::{Dht, DhtFactory};

/// While the server node is suspended, `connect` ends with an error and nothing reaches the incoming queue; after `resume` the connection goes through.
pub async fn suspend_resume<F: DhtFactory>(factory: &F) {
    let mut server = listen_on(factory, key_pair(0x61), None).await;
    let client = node(factory, 0x62).await;
    server.dht.suspend().await.expect("suspend: suspend");
    connect_refused(&client, server.pk, "suspend: while suspended").await;
    assert_empty(&mut server.incoming, "suspend: while suspended").await;
    server.dht.resume().await.expect("suspend: resume");
    let client_pk = key_pair(0x62).public_key;
    connect_pair(&client, client_pk, &mut server, "suspend: after resume").await;
}
