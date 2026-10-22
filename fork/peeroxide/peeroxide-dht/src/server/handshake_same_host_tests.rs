// JS: hyperdht/lib/server.js:440-445
//! Cluster tests of the same-host rule on the handshake path: a relayed client that sits behind the
//! server's own public IP is answered from the server's node socket (no puncher); any other client
//! keeps the puncher. The cluster is on 127.0.0.1, so "the same host" is the loopback address and
//! "another host" is a node bound to this machine's LAN address. Included from `handshake_tests.rs`.

use super::*;
use crate::hyperdht::{ConnectOpts, PeerConnection};
use crate::noise_wrap::NoiseWrap;
use crate::server::test_net::{hello_with, lan_host, node_on, settled_remote_address};

mod loopback_cluster_firewalled_server {
    use super::*;

    #[tokio::test]
    async fn server_learns_its_loopback_address() {
        within(async {
            // given: a firewalled server in a loopback cluster
            let cl = cluster(3).await;
            let server = server_on(&cl, true, ServerOpts::default()).await;
            // when: its own address settles
            let ours = settled_remote_address(&server.handle).await;
            // then: the relays see it at 127.0.0.1 (otherwise the tests below prove nothing)
            assert_eq!(
                ours.host, "127.0.0.1",
                "server address as the relays see it"
            );
        })
        .await;
    }

    #[tokio::test]
    async fn lan_client_is_answered_from_the_node_socket() {
        within(async {
            // given: both addresses settled; the client announces its own address and a LAN one
            let fx = setup(Opts::same_host()).await;
            let announced = vec![fx.client_remote.clone(), peer("192.168.1.5", 1)];
            // when: a relayed hello
            let answer = fx.when_hello(announced).await;
            // then: the reply leaves from the node socket and announces only that socket
            fx.then_answered_from_node_socket(&answer);
        })
        .await;
    }

    #[tokio::test]
    async fn lan_client_repeat_is_answered_from_the_same_socket() {
        within(async {
            // given: the client announces its own address and a LAN one
            let fx = setup(Opts::same_host()).await;
            let announced = vec![fx.client_remote.clone(), peer("192.168.1.5", 1)];
            // when: the same Noise message arrives through another relay
            let (first, repeat) = fx.when_hello_repeated(announced).await;
            // then: the repeat is answered from the same socket
            assert_eq!(repeat, first, "the repeat left from another socket");
        })
        .await;
    }

    #[tokio::test]
    async fn lone_reflexive_client_gets_the_puncher() {
        within(async {
            // given: the client announces only its own address (no LAN address)
            let fx = setup(Opts::same_host()).await;
            let announced = vec![fx.client_remote.clone()];
            // when: a relayed hello
            let answer = fx.when_hello(announced).await;
            // then: the default behaviour, a puncher
            fx.then_answered_by_puncher(&answer);
        })
        .await;
    }

    #[tokio::test]
    async fn client_without_own_host_gets_the_puncher() {
        within(async {
            // given: the client announces only a LAN address, not an address with its own host
            let fx = setup(Opts::same_host()).await;
            let announced = vec![peer("192.168.1.5", 1)];
            // when: a relayed hello
            let answer = fx.when_hello(announced).await;
            // then: the default behaviour, a puncher
            fx.then_answered_by_puncher(&answer);
        })
        .await;
    }
}

/// The client node is bound to this machine's LAN address: the relays (and the server) see it at
/// another host than the server's `127.0.0.1`.
mod client_from_another_host {
    use super::*;

    #[tokio::test]
    async fn other_host_client_gets_the_puncher() {
        within(async {
            // given: both addresses settled; the client announces its own address and a LAN one
            // (conditions (b) and (c) hold, (a) does not)
            let fx = setup(Opts::other_host()).await;
            let announced = vec![fx.client_remote.clone(), peer("10.9.9.9", 1)];
            // when: a relayed hello
            let answer = fx.when_hello(announced).await;
            // then: a puncher, as upstream does
            fx.then_answered_by_puncher(&answer);
        })
        .await;
    }

    #[tokio::test]
    async fn other_host_client_connects_by_holepunch() {
        within(async {
            // given: a client on another host with default connect options
            let mut fx = setup(Opts::other_host()).await;
            // when: it connects
            let conn = fx.when_connect().await;
            // then: both ends know each other
            assert_eq!(conn.remote_public_key, key(1).public_key, "client side");
            let accepted = fx.accepted().await;
            assert_eq!(accepted.remote_public_key, key(2).public_key, "server side");
        })
        .await;
    }
}

/// Scenario preconditions.
struct Opts {
    /// The client node is bound to the LAN address instead of 127.0.0.1.
    client_on_lan_address: bool,
}

impl Opts {
    /// The client shares 127.0.0.1 with the server: the relay sees both at one host.
    fn same_host() -> Opts {
        Opts {
            client_on_lan_address: false,
        }
    }

    /// The client sits at the LAN address: the relay sees another host.
    fn other_host() -> Opts {
        Opts {
            client_on_lan_address: true,
        }
    }
}

/// A cluster, a firewalled server and a client node; both own addresses have settled.
struct Fixture {
    cl: Cluster,
    server: Served,
    client: Node,
    /// The client's address as the relays see it.
    client_remote: Ipv4Peer,
    /// Port of the server's node socket.
    node_port: u16,
    /// Port of the server's listen socket.
    listen_port: u16,
}

/// What a relayed hello got back: the port the reply left from, and the server's `addresses4`.
struct Answer {
    port: u16,
    addresses4: Vec<Ipv4Peer>,
}

impl Fixture {
    /// A relayed hello announcing `announced`, sent through the first cluster node.
    async fn when_hello(&self, announced: Vec<Ipv4Peer>) -> Answer {
        let (mut nw, noise) = self.hello_announcing(announced);
        let reply = self.when_relayed(&noise, 0).await;
        let payload = nw.recv(&reply.noise).expect("server payload");
        Answer {
            port: reply.server_address.port,
            addresses4: payload.addresses4,
        }
    }

    /// The same Noise message through two different relays: the reply port of the first and of the repeat.
    async fn when_hello_repeated(&self, announced: Vec<Ipv4Peer>) -> (u16, u16) {
        let (_nw, noise) = self.hello_announcing(announced);
        let first = self.when_relayed(&noise, 0).await;
        let repeat = self.when_relayed(&noise, 1).await;
        (first.server_address.port, repeat.server_address.port)
    }

    /// The client connects with default options (`local_connection: true`).
    async fn when_connect(&self) -> PeerConnection {
        let connected = self
            .client
            .handle
            .connect_with_options(
                &key(2),
                key(1).public_key,
                &[],
                &self.client.rt,
                ConnectOpts::default(),
            )
            .await;
        connected.expect("connects by holepunch")
    }

    /// The next connection the server accepted (10 s at most).
    async fn accepted(&mut self) -> PeerConnection {
        let next = tokio::time::timeout(Duration::from_secs(10), self.server.incoming.recv()).await;
        next.expect("no incoming connection in 10 s")
            .expect("incoming channel closed")
    }

    fn then_answered_from_node_socket(&self, answer: &Answer) {
        assert_eq!(
            answer.port, self.node_port,
            "reply left from the node socket"
        );
        assert!(
            !answer.addresses4.is_empty(),
            "the reply announces addresses"
        );
        for address in &answer.addresses4 {
            assert_eq!(address.port, self.node_port, "announced {address:?}");
        }
    }

    fn then_answered_by_puncher(&self, answer: &Answer) {
        let ports = std::iter::once(answer.port).chain(answer.addresses4.iter().map(|a| a.port));
        for port in ports {
            assert_ne!(port, self.node_port, "the node socket is not a puncher");
            assert_ne!(port, self.listen_port, "the listen socket is not a puncher");
        }
    }

    fn hello_announcing(&self, announced: Vec<Ipv4Peer>) -> (NoiseWrap, Vec<u8>) {
        let (client, server_pk) = (key(2), key(1).public_key);
        hello_with(&client, server_pk, FIREWALL_UNKNOWN, Some(7), announced)
    }

    /// The server's answer to `noise` relayed by cluster node number `relay`.
    async fn when_relayed(&self, noise: &[u8], relay: usize) -> HandshakeResult {
        let to = &self.cl.addrs[relay];
        let reply = relayed(&self.client, noise, to, None).await;
        reply.expect("server replies")
    }
}

async fn setup(o: Opts) -> Fixture {
    let cl = cluster(3).await;
    let server = server_on(&cl, true, ServerOpts::default()).await;
    let client = if o.client_on_lan_address {
        node_on(&cl, &lan_host(), true).await
    } else {
        node(&cl, true).await
    };
    settled_remote_address(&server.handle).await;
    let client_remote = settled_remote_address(&client.handle).await;
    if o.client_on_lan_address {
        let lan = lan_host();
        assert_eq!(
            client_remote.host, lan,
            "the relays must see the client at the LAN address"
        );
    }
    let node_port = primary_port(&server.handle).await;
    let listen_port = server.handle.local_port().await.expect("local_port");
    Fixture {
        cl,
        server,
        client,
        client_remote,
        node_port,
        listen_port,
    }
}

fn peer(host: &str, port: u16) -> Ipv4Peer {
    Ipv4Peer {
        host: host.into(),
        port,
    }
}
