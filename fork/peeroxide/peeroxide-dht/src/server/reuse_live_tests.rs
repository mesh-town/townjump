// JS: hyperdht/lib/connect.js (reusableSocket), hyperdht/lib/server.js
//! Live tests on a local cluster: the client's second connection to the same server goes over the
//! route of the first (holepunch socket + server address), not a new holepunch. Both nodes are by default
//! firewalled (holepunch), `local_connection: false`, `reusable_socket: true` at the client and server.
//! Real time (the network is live, tokio pause is not used): the `LINGER` delay is waited out with
//! a real `sleep`. Related: `handle_tests.rs` (model), `test_net.rs` (cluster and helpers).

use std::time::Duration;

use libudx::UdxSocket;

use super::ServerOpts;
use super::test_net::{
    Cluster, Node, Served, cluster, echo, key, lan_host, node, roundtrip, serve,
    settled_remote_address, sha256, within,
};
use crate::hyperdht::{ConnectOpts, KeyPair, LINGER, PeerConnection};

mod first_connection {
    use super::*;

    mod both_sides_reusable {
        use super::*;

        #[tokio::test]
        async fn holds_the_punch_socket_on_both_sides() {
            within(async {
                // given: a client and a server with reusable_socket
                let mut fx = setup(Opts::default()).await;
                // when: the first connection
                let first = fx.when_connect().await;
                // then: both hold the holepunch socket (the lease is tied to the connection)
                assert!(
                    fx.client_held(&first.client.socket),
                    "client does not hold S"
                );
                assert!(
                    fx.server_held(&first.server.socket),
                    "server does not hold S'"
                );
            })
            .await;
        }
    }

    mod client_without_option {
        use super::*;

        #[tokio::test]
        async fn nothing_is_held() {
            within(async {
                // given: the client has reusable_socket: false
                let mut fx = setup(Opts::client_off()).await;
                // when: the first connection
                let first = fx.when_connect().await;
                // then: nobody holds: the client did not announce, the server does not ask
                assert!(!fx.client_held(&first.client.socket), "client holds S");
                assert!(!fx.server_held(&first.server.socket), "server holds S'");
            })
            .await;
        }
    }

    mod server_without_option {
        use super::*;

        #[tokio::test]
        async fn nothing_is_held() {
            within(async {
                // given: the server has reusable_socket: false
                let mut fx = setup(Opts::server_off()).await;
                // when: the first connection
                let first = fx.when_connect().await;
                // then: nobody holds: the server did not announce, the client sees no consent
                assert!(!fx.client_held(&first.client.socket), "client holds S");
                assert!(!fx.server_held(&first.server.socket), "server holds S'");
            })
            .await;
        }
    }
}

#[path = "reuse_live_second_tests.rs"]
mod second_connection;

#[path = "reuse_live_route_tests.rs"]
mod route;

/// Scenario preconditions.
struct Opts {
    client_reusable: bool,
    server_reusable: bool,
    /// Nodes not firewalled and `local_connection` default (direct branch on the IO socket);
    /// otherwise both firewalled and `local_connection: false` (holepunch).
    direct: bool,
    /// Both firewalled (holepunch) and `local_connection: true`: the client announces its LAN
    /// addresses, and the server sees it behind its own IP (a loopback cluster).
    lan: bool,
}

impl Default for Opts {
    fn default() -> Self {
        Opts {
            client_reusable: true,
            server_reusable: true,
            direct: false,
            lan: false,
        }
    }
}

impl Opts {
    fn client_off() -> Opts {
        Opts {
            client_reusable: false,
            ..Opts::default()
        }
    }

    fn server_off() -> Opts {
        Opts {
            server_reusable: false,
            ..Opts::default()
        }
    }

    fn direct() -> Opts {
        Opts {
            direct: true,
            ..Opts::default()
        }
    }

    fn lan() -> Opts {
        Opts {
            lan: true,
            ..Opts::default()
        }
    }
}

/// Cluster, a listening server and a client node.
struct Fixture {
    _cluster: Cluster,
    client: Node,
    server: Served,
    client_key: KeyPair,
    server_key: KeyPair,
    connect_opts: ConnectOpts,
}

/// Two ends of one connection.
struct Pair {
    client: PeerConnection,
    server: PeerConnection,
}

/// Pair sockets (clones: `ptr_eq` works after the connections close too).
struct Sockets {
    client: UdxSocket,
    server: UdxSocket,
}

impl Pair {
    fn sockets(&self) -> Sockets {
        Sockets {
            client: self.client.socket.clone(),
            server: self.server.socket.clone(),
        }
    }
}

impl Fixture {
    /// The client's connection to the server and the one accepted by the server.
    async fn when_connect(&mut self) -> Pair {
        let client = self
            .client
            .handle
            .connect_with_options(
                &self.client_key,
                self.server_key.public_key,
                &[],
                &self.client.rt,
                self.connect_opts.clone(),
            )
            .await
            .expect("connect");
        let server = accepted(&mut self.server).await;
        Pair { client, server }
    }

    fn hits(&self) -> u64 {
        self.client.handle.reuse.hits()
    }

    fn drops(&self) -> u64 {
        self.client.handle.reuse.drops()
    }

    fn client_held(&self, socket: &UdxSocket) -> bool {
        self.client.handle.reuse.is_held(socket)
    }

    fn server_held(&self, socket: &UdxSocket) -> bool {
        self.server.handle.reuse.is_held(socket)
    }

    /// The socket the client node sends DHT requests from.
    async fn client_io_socket(&self) -> UdxSocket {
        let socket = self.client.handle.server_socket().await;
        socket.expect("server_socket").expect("primary socket")
    }

    /// The socket the server node sends DHT requests from (its node IO socket).
    async fn server_io_socket(&self) -> UdxSocket {
        let socket = self.server.handle.server_socket().await;
        socket.expect("server_socket").expect("primary socket")
    }
}

async fn setup(o: Opts) -> Fixture {
    let cluster = cluster(3).await;
    let firewalled = !o.direct;
    let server_key = key(1);
    let server_opts = ServerOpts {
        reusable_socket: o.server_reusable,
        ..ServerOpts::default()
    };
    let server = serve(node(&cluster, firewalled).await, &server_key, server_opts).await;
    let client = node(&cluster, firewalled).await;
    let connect_opts = ConnectOpts {
        local_connection: o.direct || o.lan,
        reusable_socket: o.client_reusable,
        ..ConnectOpts::default()
    };
    if o.lan {
        settle_lan(&server, &client).await;
    }
    Fixture {
        _cluster: cluster,
        client,
        server,
        client_key: key(2),
        server_key,
        connect_opts,
    }
}

/// The `lan` scenario needs a non-loopback interface (without it the client announces no private
/// address and the same-host rule cannot hold) and both nodes' own addresses settled.
async fn settle_lan(server: &Served, client: &Node) {
    let lan = lan_host();
    let ours = settled_remote_address(&server.handle).await;
    let theirs = settled_remote_address(&client.handle).await;
    assert_eq!(ours.host, theirs.host, "both nodes sit behind one host");
    assert_ne!(
        ours.host, lan,
        "the LAN address must differ from the shared host"
    );
}

/// The next connection the server accepted (10 s at most).
async fn accepted(server: &mut Served) -> PeerConnection {
    let next = tokio::time::timeout(Duration::from_secs(10), server.incoming.recv()).await;
    next.expect("no incoming connection in 10 s")
        .expect("incoming channel closed")
}

/// Echo through the pair: the server returns everything it received; `true` if the bytes came back intact.
async fn echoes(pair: Pair) -> bool {
    let bytes: Vec<u8> = (0..150_000usize).map(|i| (i % 251) as u8).collect();
    let expected = sha256(&bytes);
    let server_end = tokio::spawn(echo(pair.server));
    let got = roundtrip(pair.client, bytes).await;
    let _ = server_end.await;
    got == expected
}
