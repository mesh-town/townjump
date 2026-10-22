//! Cluster tests of the handshake path: what the server answers to a raw
//! PEER_HANDSHAKE, and who sends the first UDX packet. Each test runs a local
//! DHT cluster in the process.

use std::net::IpAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use libudx::UdxStream;
use tokio::sync::oneshot;

use crate::hyperdht::alloc_stream_id;
use crate::hyperdht_messages::{FIREWALL_OPEN, FIREWALL_UNKNOWN};
use crate::messages::Ipv4Peer;
use crate::router::HandshakeResult;
use crate::server::test_net::{
    Cluster, Node, Served, cluster, hello, key, node, primary_port, raw_handshake,
    raw_handshake_via, serve, within,
};
use crate::server::{Firewall, ServerOpts};

/// Server options whose firewall counts its calls and accepts everybody.
fn counting_opts() -> (ServerOpts, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&calls);
    let firewall: Firewall = Arc::new(move |_| {
        seen.fetch_add(1, Ordering::SeqCst);
        false
    });
    (ServerOpts::default().with_firewall(firewall), calls)
}

/// A server on a new node (`firewalled` as asked), announced to the cluster.
async fn server_on(cl: &Cluster, firewalled: bool, opts: ServerOpts) -> Served {
    serve(node(cl, firewalled).await, &key(1), opts).await
}

/// The client's UDX stream with a hook on its primary socket: the receiver
/// gets the source of the first packet that arrives for `cid`.
async fn expect_first_packet(
    client: &Node,
    cid: u32,
    server_udx_id: u32,
) -> (UdxStream, oneshot::Receiver<(IpAddr, u16)>) {
    let stream = client.rt.create_stream(cid).await.expect("client stream");
    let socket = client.handle.server_socket().await.expect("socket");
    let socket = socket.expect("primary socket");
    let (tx, rx) = oneshot::channel();
    let hook = move |_: &libudx::UdxSocket, port: u16, ip: IpAddr| {
        let _ = tx.send((ip, port));
        true
    };
    stream
        .set_firewall_hook(&socket, server_udx_id, hook)
        .expect("firewall hook");
    (stream, rx)
}

/// Sends a handshake claiming `firewall` and a UDX stream `cid`; returns the
/// UDX id from the server's reply.
async fn shake(client: &Node, to: &crate::messages::Ipv4Peer, firewall: u64, cid: u32) -> u32 {
    let server_pk = key(1).public_key;
    let (mut nw, noise) = hello(&key(2), server_pk, firewall, Some(u64::from(cid)));
    let res = raw_handshake(client, server_pk, &noise, to).await;
    let payload = nw.recv(&res.expect("server replies").noise);
    let udx = payload.expect("server payload").udx.expect("server udx");
    u32::try_from(udx.id).expect("udx id fits u32")
}

/// Waits for the first packet of the client's stream; returns its source port.
async fn first_packet_port(arrived: oneshot::Receiver<(IpAddr, u16)>) -> u16 {
    let wait = tokio::time::timeout(Duration::from_secs(5), arrived).await;
    let (ip, port) = wait
        .expect("the server sent no packet in 5 s")
        .expect("hook");
    assert_eq!(ip, IpAddr::from([127, 0, 0, 1]));
    port
}

#[tokio::test]
async fn firewall_called_once_per_repeated_handshake() {
    within(async {
        let cl = cluster(3).await;
        let (opts, calls) = counting_opts();
        let _server = server_on(&cl, false, opts).await;
        let client = node(&cl, false).await;
        let (_nw, noise) = hello(&key(2), key(1).public_key, FIREWALL_UNKNOWN, Some(7));
        let first = raw_handshake(&client, key(1).public_key, &noise, &cl.addrs[0]).await;
        let second = raw_handshake(&client, key(1).public_key, &noise, &cl.addrs[1]).await;
        let (first, second) = (first.expect("first reply"), second.expect("second reply"));
        assert_eq!(first.noise, second.noise, "a repeat gets the stored reply");
        assert_eq!(calls.load(Ordering::SeqCst), 1, "firewall called once");
    })
    .await;
}

#[tokio::test]
async fn holepunch_reply_peer_address_is_relay_view() {
    within(async {
        let cl = cluster(3).await;
        let server = server_on(&cl, true, ServerOpts::default()).await;
        let client = node(&cl, true).await;
        let server_pk = key(1).public_key;
        let (mut nw, noise) = hello(&key(2), server_pk, FIREWALL_UNKNOWN, Some(7));
        let res = raw_handshake(&client, server_pk, &noise, &cl.addrs[0]).await;
        let payload = nw
            .recv(&res.expect("server replies").noise)
            .expect("payload");
        let relays = payload
            .holepunch
            .expect("a firewalled server punches")
            .relays;
        assert!(!relays.is_empty(), "the reply carries relays");
        let ours = primary_port(&server.handle).await;
        let theirs = primary_port(&client.handle).await;
        let listen = server.handle.local_port().await.expect("local_port");
        for relay in &relays {
            assert_eq!(
                relay.peer_address.port, ours,
                "server address as the relay sees it"
            );
            assert_ne!(relay.peer_address.port, theirs, "not the client's address");
            assert_ne!(relay.peer_address.port, listen, "not the listen socket");
        }
    })
    .await;
}

#[tokio::test]
async fn announce_relays_carry_our_address_as_seen_by_relay() {
    within(async {
        let cl = cluster(3).await;
        let announcer = node(&cl, true).await;
        let kp = key(1);
        let target = crate::crypto::hash(&kp.public_key);
        let res = announcer.handle.announce(target, &kp, &[]).await;
        let relays = res.expect("announce").relays;
        assert!(!relays.is_empty(), "announce reports its relays");
        let ours = primary_port(&announcer.handle).await;
        for relay in &relays {
            assert_eq!(
                relay.peer_address.port, ours,
                "our address as the relay sees it"
            );
        }
    })
    .await;
}

#[tokio::test]
async fn open_client_is_dialed_by_server() {
    within(async {
        let cl = cluster(3).await;
        let server = server_on(&cl, true, ServerOpts::default()).await;
        let client = node(&cl, false).await;
        let cid = alloc_stream_id();
        let sid = shake(&client, &cl.addrs[0], FIREWALL_OPEN, cid).await;
        let (_stream, arrived) = expect_first_packet(&client, cid, sid).await;
        let port = first_packet_port(arrived).await;
        assert_eq!(
            port,
            primary_port(&server.handle).await,
            "dialed from the primary socket"
        );
    })
    .await;
}

/// A direct handshake to the server's listen port; returns that port and the source
/// port of the server's first UDX packet.
async fn direct_dial(server_firewalled: bool) -> (u16, u16) {
    let cl = cluster(3).await;
    let server = server_on(&cl, server_firewalled, ServerOpts::default()).await;
    let client = node(&cl, false).await;
    let listen = server.handle.local_port().await.expect("local_port");
    let to = Ipv4Peer {
        host: "127.0.0.1".into(),
        port: listen,
    };
    let cid = alloc_stream_id();
    let sid = shake(&client, &to, FIREWALL_UNKNOWN, cid).await;
    let (_stream, arrived) = expect_first_packet(&client, cid, sid).await;
    (listen, first_packet_port(arrived).await)
}

#[tokio::test]
async fn direct_request_is_dialed() {
    let (listen, from) = within(direct_dial(false)).await;
    assert_eq!(from, listen);
}

/// JS: a direct request is answered and dialled on the socket it came in on, and that is
/// the listen socket even when the node is firewalled.
#[tokio::test]
async fn direct_request_to_firewalled_server_is_dialed_from_listen_port() {
    let (listen, from) = within(direct_dial(true)).await;
    assert_eq!(from, listen);
}

/// A relayed handshake (`relay`: the second relay named by the client); the reply and
/// the server's own view of it: `server_address` is where the holder saw FROM_SERVER come from.
async fn relayed(
    client: &Node,
    noise: &[u8],
    to: &Ipv4Peer,
    relay: Option<&Ipv4Peer>,
) -> Option<HandshakeResult> {
    let server_pk = key(1).public_key;
    raw_handshake_via(client, server_pk, noise, to, relay.cloned()).await
}

/// JS (probe, open server, FROM_RELAY): the answer leaves from the listen socket.
#[tokio::test]
async fn open_server_relayed_answer_leaves_from_listen_port() {
    within(async {
        let cl = cluster(3).await;
        let server = server_on(&cl, false, ServerOpts::default()).await;
        let client = node(&cl, false).await;
        let (_nw, noise) = hello(&key(2), key(1).public_key, FIREWALL_UNKNOWN, Some(7));
        let res = relayed(&client, &noise, &cl.addrs[0], None).await;
        let listen = server.handle.local_port().await.expect("local_port");
        assert_eq!(res.expect("reply").server_address.port, listen);
    })
    .await;
}

/// JS (probe, firewalled server without LAN shortcut, FROM_RELAY): the answer leaves from
/// the hole-punch socket, a third port; a repeat of the same Noise message uses it again.
#[tokio::test]
async fn holepunch_relayed_answer_leaves_from_puncher() {
    within(async {
        let cl = cluster(3).await;
        let server = server_on(&cl, true, ServerOpts::default()).await;
        let client = node(&cl, true).await;
        let (_nw, noise) = hello(&key(2), key(1).public_key, FIREWALL_UNKNOWN, Some(7));
        let first = relayed(&client, &noise, &cl.addrs[0], None).await;
        let repeat = relayed(&client, &noise, &cl.addrs[1], None).await;
        let (first, repeat) = (first.expect("reply"), repeat.expect("repeat reply"));
        let listen = server.handle.local_port().await.expect("local_port");
        let primary = primary_port(&server.handle).await;
        let port = first.server_address.port;
        assert_ne!(port, listen, "not the listen socket");
        assert_ne!(port, primary, "not the client socket");
        assert_eq!(repeat.server_address.port, port, "the repeat: same puncher");
    })
    .await;
}

/// JS (probe, FROM_SECOND_RELAY): the server answers `relayAddress` (the first relay),
/// not the node that forwarded the request; without it the client gets no reply.
async fn second_relay_reply(server_firewalled: bool) {
    let cl = cluster(3).await;
    let _server = server_on(&cl, server_firewalled, ServerOpts::default()).await;
    let client = node(&cl, server_firewalled).await;
    let (_nw, noise) = hello(&key(2), key(1).public_key, FIREWALL_UNKNOWN, Some(7));
    let res = relayed(&client, &noise, &cl.addrs[0], Some(&cl.addrs[1])).await;
    assert!(res.is_some(), "the first relay got the answer and replied");
}

#[tokio::test]
async fn open_server_second_relay_answers_relay_address() {
    within(second_relay_reply(false)).await;
}

#[tokio::test]
async fn firewalled_server_second_relay_answers_relay_address() {
    within(second_relay_reply(true)).await;
}

#[path = "handshake_same_host_tests.rs"]
mod same_host;

#[path = "netchange_lab_tests.rs"]
mod netchange_lab;
