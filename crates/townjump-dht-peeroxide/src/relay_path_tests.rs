// Tests of the relay path label on live connections: both ends carry it, it follows the bridge to the direct path,
// and a connection without `relay_through` has none. Run on the in-process native network of the fork.
use crate::{PeeroxideRelay, RelayOpts};
use std::any::Any;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use townjump_dht::{
    ConnectOpts, Dht, DhtConnection, DhtFactory, DhtServer, RelayAddress, RelayPath, RelayThrough,
    ServerOpts,
};
use townjump_testkit::conformance::wait::{ACCEPT_WAIT, STREAM_WAIT, within};
use townjump_testkit::key_pair;

// One helper file is included in several test files: a separate copy in each would be pointless.
#[allow(clippy::duplicate_mod)]
#[path = "native_testnet_tests.rs"]
pub(super) mod native;
use native::{NativeTestnet, native_testnet, relay_only};

const RELAY_TAG: u8 = 0xa1;
const SERVER_TAG: u8 = 0xa2;
const CLIENT_TAG: u8 = 0xa3;
const NETWORK_SIZE: usize = 4;
const PUNCH_DELAY: Duration = Duration::from_secs(2);
const DIRECT_WAIT: Duration = Duration::from_secs(10);
const POLL_STEP: Duration = Duration::from_millis(200);
const POLL_TRIES: usize = 50;
const PAYLOAD: usize = 2048;
const ECHO_ROUNDS: usize = 3;

mod relay_through_a_relay {
    use super::*;

    mod both_sides_firewalled {
        use super::*;

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn both_ends_are_labelled_relayed_on_connect() {
            let scene = setup(Want::through_relay().punching_after(PUNCH_DELAY)).await;

            assert!(scene.client.is_relayed(), "client end");
            assert!(scene.accepted.is_relayed(), "end taken from Incoming");
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn both_ends_switch_to_direct_after_the_holepunch() {
            let scene = setup(Want::through_relay().punching_after(PUNCH_DELAY)).await;

            let client = wait_direct(&scene.client, "client end").await;
            let accepted = wait_direct(&scene.accepted, "accepted end").await;

            assert!(client && accepted, "client {client}, accepted {accepted}");
            assert!(!scene.client.is_relayed());
            assert!(!scene.accepted.is_relayed());
        }
    }

    mod direct_path_forbidden {
        use super::*;

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn both_ends_stay_relayed_through_the_whole_echo() {
            let mut scene = setup(Want::through_relay().relay_only()).await;

            for round in 0..ECHO_ROUNDS {
                scene.echo_round().await;
                assert!(scene.client.is_relayed(), "client end, round {round}");
                assert!(scene.accepted.is_relayed(), "accepted end, round {round}");
            }
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn the_label_never_announces_a_change() {
            let mut scene = setup(Want::through_relay().relay_only()).await;

            scene.echo_round().await;
            tokio::time::sleep(PUNCH_DELAY).await;

            for (end, conn) in [("client", &scene.client), ("accepted", &scene.accepted)] {
                let rx = conn.relay_path().expect("labelled");
                assert!(!rx.has_changed().unwrap(), "{end} end changed");
                assert_eq!(*rx.borrow(), RelayPath::Relayed, "{end} end");
            }
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn the_relay_matched_a_pairing() {
            let mut scene = setup(Want::through_relay().relay_only()).await;

            scene.echo_round().await;

            let matched = scene.relay_matched_a_pairing().await;

            assert!(matched, "stats: {:?}", scene.relay.stats());
        }
    }
}

mod without_relay_through {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn neither_end_has_a_path_label() {
        let scene = setup(Want::direct()).await;

        assert!(scene.client.relay_path().is_none(), "client end");
        assert!(scene.accepted.relay_path().is_none(), "accepted end");
        assert!(!scene.client.is_relayed());
        assert!(!scene.accepted.is_relayed());
    }
}

struct Want {
    relayed: bool,
    relay_only: bool,
    punch_delay: Option<Duration>,
}

impl Want {
    fn through_relay() -> Self {
        Want {
            relayed: true,
            relay_only: false,
            punch_delay: None,
        }
    }

    fn direct() -> Self {
        Want {
            relayed: false,
            ..Self::through_relay()
        }
    }

    fn relay_only(self) -> Self {
        Want {
            relay_only: true,
            ..self
        }
    }

    fn punching_after(self, delay: Duration) -> Self {
        Want {
            punch_delay: Some(delay),
            ..self
        }
    }
}

/// Both ends of one connection plus everything that has to stay alive for it.
struct Scene {
    relay: PeeroxideRelay,
    client: DhtConnection,
    accepted: DhtConnection,
    _alive: Vec<Box<dyn Any + Send>>,
}

impl Scene {
    /// Client writes, the accepted end reads it back and writes it again, the client reads the echo.
    async fn echo_round(&mut self) {
        let sent: Vec<u8> = (0..PAYLOAD).map(|i| (i % 251) as u8).collect();
        self.client.stream.write_all(&sent).await.expect("write");
        let mut got = vec![0u8; PAYLOAD];
        let server = &mut self.accepted.stream;
        let read = within(STREAM_WAIT, "server read", server.read_exact(&mut got));
        read.await.expect("server read");
        server.write_all(&got).await.expect("server write");
        let mut echoed = vec![0u8; PAYLOAD];
        let client = &mut self.client.stream;
        let back = within(STREAM_WAIT, "echo", client.read_exact(&mut echoed));
        back.await.expect("read of the echo");
        assert_eq!(sent, echoed, "echo differs from what was written");
    }

    /// Polls the relay counter until a pairing is matched or ~10 s pass.
    async fn relay_matched_a_pairing(&self) -> bool {
        for _ in 0..POLL_TRIES {
            if self.relay.stats().pairings_matched >= 1 {
                return true;
            }
            tokio::time::sleep(POLL_STEP).await;
        }
        false
    }
}

/// Starts a relay, a firewalled server and a firewalled client; the client connects as `want` says.
#[allow(clippy::too_many_lines)] // one linear scenario: relay, server, client, connect
async fn setup(want: Want) -> Scene {
    let testnet: NativeTestnet = native_testnet(NETWORK_SIZE).await;
    let relay = start_relay(&testnet).await;
    let address = relay_address(&relay).await;
    let (server_dht, server, mut incoming) = match want.relayed {
        true => {
            let dht = testnet.firewalled_dht(Some(key_pair(SERVER_TAG))).await;
            let opts = ServerOpts {
                relay_addresses: vec![address],
                ..Default::default()
            };
            let (server, incoming) = dht.create_server(opts).expect("create_server");
            (dht, server, incoming)
        }
        false => {
            let node = testnet.factory.create(Some(key_pair(SERVER_TAG)));
            let dht = within(ACCEPT_WAIT, "server node", node)
                .await
                .expect("node");
            let opts = ServerOpts::default();
            let (server, incoming) = dht.create_server(opts).expect("create_server");
            (dht, server, incoming)
        }
    };
    let listen = within(ACCEPT_WAIT, "listen", server.listen(key_pair(SERVER_TAG)));
    listen.await.expect("listen");
    let client_dht = match want.relayed {
        true => testnet.firewalled_dht(Some(key_pair(CLIENT_TAG))).await,
        false => {
            let dht = testnet.factory.create(Some(key_pair(CLIENT_TAG)));
            within(ACCEPT_WAIT, "client node", dht).await.expect("node")
        }
    };
    let mut opts = ConnectOpts::default();
    if want.relayed {
        opts.relay_through = Some(RelayThrough::Key(relay.public_key()));
        opts.relay_addresses = vec![address];
        opts.local_connection = false;
        opts.punch_delay = want.punch_delay;
    }
    if want.relay_only {
        relay_only(&mut opts);
    }
    let connect = client_dht.connect(key_pair(SERVER_TAG).public_key, opts);
    let client = within(ACCEPT_WAIT, "connect", connect).await;
    let client = client.expect("connect");
    let accepted = within(ACCEPT_WAIT, "accept", incoming.accept()).await;
    let accepted = accepted.expect("the server got the connection");
    let alive: Vec<Box<dyn Any + Send>> = vec![
        Box::new(testnet),
        Box::new(server_dht),
        Box::new(server),
        Box::new(incoming),
        Box::new(client_dht),
    ];
    Scene {
        relay,
        client,
        accepted,
        _alive: alive,
    }
}

async fn start_relay(testnet: &NativeTestnet) -> PeeroxideRelay {
    let opts = RelayOpts {
        identity: Some(key_pair(RELAY_TAG)),
        ..Default::default()
    };
    let started = testnet.relay_try(opts);
    within(ACCEPT_WAIT, "create_relay", started)
        .await
        .expect("create_relay_with")
}

/// The relay key with its address, once the relay knows it (`assume_open` on the loopback: right after the start).
async fn relay_address(relay: &PeeroxideRelay) -> RelayAddress {
    for _ in 0..POLL_TRIES {
        if let Ok(Some(address)) = relay.relay_address().await {
            return address;
        }
        tokio::time::sleep(POLL_STEP).await;
    }
    panic!("the relay never learned its address");
}

/// True when the end reaches `Direct` within `DIRECT_WAIT`; the end must be labelled.
async fn wait_direct(conn: &DhtConnection, end: &str) -> bool {
    let mut rx = conn
        .relay_path()
        .unwrap_or_else(|| panic!("{end}: no label"));
    let reached = async { rx.wait_for(|p| *p == RelayPath::Direct).await.is_ok() };
    let done = tokio::time::timeout(DIRECT_WAIT, reached).await;
    done.unwrap_or(false)
}
