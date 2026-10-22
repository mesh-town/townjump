// JS: streamx Duplex of a hyperdht connection: a megabyte both ways and closing the write side (`end`).
// Related: setup.rs (nodes, `connect_pair`), data.rs (exchange and pattern). Keys: 0x11/0x12 and 0x21/0x22.
use super::data::{exchange, read_all, send_and_close};
use super::keys::key_pair;
use super::setup::{connect_pair, listen_on, node};
use townjump_dht::DhtFactory;

const MIB: usize = 1 << 20;

/// A megabyte both ways at once; bytes are equal, identities on both sides are correct.
pub async fn stream_1mb<F: DhtFactory>(factory: &F) {
    let mut server = listen_on(factory, key_pair(0x11), None).await;
    let client = node(factory, 0x12).await;
    let client_pk = key_pair(0x12).public_key;
    let (dialed, accepted) = connect_pair(&client, client_pk, &mut server, "stream_1mb").await;
    exchange(dialed.stream, accepted.stream, MIB, "stream_1mb").await;
}

/// The client writes `ping` and closes the write side; the server sees `EOF`, replies `pong` and closes its own.
pub async fn half_close<F: DhtFactory>(factory: &F) {
    let mut server = listen_on(factory, key_pair(0x21), None).await;
    let client = node(factory, 0x22).await;
    let client_pk = key_pair(0x22).public_key;
    let (dialed, accepted) = connect_pair(&client, client_pk, &mut server, "half_close").await;
    let (mut from_client, mut from_server) = (dialed.stream, accepted.stream);
    send_and_close(&mut from_client, b"ping").await;
    assert_eq!(
        read_all(&mut from_server, "half_close: server").await,
        b"ping"
    );
    send_and_close(&mut from_server, b"pong").await;
    assert_eq!(
        read_all(&mut from_client, "half_close: client").await,
        b"pong"
    );
}
