// Tests of `lan_allowed`: a caller that routes the connection explicitly never gets the LAN path.
// A child of lan_race_tests: shares its helpers.
use super::added::sniffer;
use super::*;
use crate::lan_race::lan_allowed;
use crate::opts_map::fork_connect_opts;
use std::net::SocketAddrV4;
use townjump_dht::{ConnectOpts, RelayAddress, RelayThrough};

const RELAY: [u8; 32] = [5; 32];

fn through_relay() -> ConnectOpts {
    ConnectOpts {
        relay_through: Some(RelayThrough::Key(RELAY)),
        ..Default::default()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn relay_through_does_not_connect_over_the_lan() {
    let _lan = LAN.lock().await;
    let port = group_port();
    let (server, _s, _inc, key) = serving(port, &lan_closed_factory()).await;
    let client = node(&lan_closed_factory(), setup(port, ProbeStart::Now)).await;
    let result = within(
        WAIT,
        "connect",
        client.connect(key.public_key, through_relay()),
    )
    .await;
    assert!(
        result.is_err(),
        "connected over the LAN despite relay_through"
    );
    let _ = tokio::join!(client.destroy(), server.destroy());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn relay_through_sends_no_probe() {
    let _lan = LAN.lock().await;
    let port = group_port();
    let sniffer = sniffer(port);
    let client = node(&lan_closed_factory(), setup(port, ProbeStart::Now)).await;
    let _ = within(WAIT, "connect", client.connect([8; 32], through_relay())).await;
    let mut buf = [0u8; 64];
    assert!(sniffer.recv(&mut buf).is_err(), "a probe was sent");
    let _ = client.destroy().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn local_connection_off_does_not_connect_over_the_lan() {
    let _lan = LAN.lock().await;
    let port = group_port();
    let (server, _s, _inc, key) = serving(port, &lan_closed_factory()).await;
    let client = node(&lan_closed_factory(), setup(port, ProbeStart::Now)).await;
    let opts = ConnectOpts {
        local_connection: false,
        ..Default::default()
    };
    let result = within(WAIT, "connect", client.connect(key.public_key, opts)).await;
    assert!(
        result.is_err(),
        "connected over the LAN with local_connection off"
    );
    let _ = tokio::join!(client.destroy(), server.destroy());
}

fn allowed(opts: ConnectOpts) -> bool {
    lan_allowed(&fork_connect_opts(opts))
}

#[test]
fn lan_allowed_only_for_opts_that_leave_the_route_open() {
    assert!(allowed(ConnectOpts::default()));
    assert!(!allowed(through_relay()));
    let off = ConnectOpts {
        local_connection: false,
        ..Default::default()
    };
    assert!(!allowed(off));
    let addresses = ConnectOpts {
        relay_addresses: vec![RelayAddress {
            public_key: RELAY,
            addr: SocketAddrV4::new([192, 0, 2, 1].into(), 9),
        }],
        ..Default::default()
    };
    assert!(allowed(addresses));
    let timed = ConnectOpts {
        holepunch: Some(std::sync::Arc::new(|_| true)),
        punch_delay: Some(Duration::from_millis(5)),
        ..Default::default()
    };
    assert!(allowed(timed));
}
