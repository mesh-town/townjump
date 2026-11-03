//! Test of a client whose observed address changes. The client socket is bound to the
//! wildcard address, so the kernel picks the source address of each packet by its destination
//! (127.0.0.1 vs the LAN address). The relay-observed server address is 127.0.0.1 in this test,
//! so the client's "old address" is 127.0.0.1 and the "new" one is the LAN address. It does not
//! cover a real NAT and its mappings, the RANDOM class of a CGNAT, an interface change event, or
//! sleep. Included from `handshake_tests.rs`.

use super::*;
use crate::hyperdht::ConnectOpts;
use crate::rpc::{DhtConfig, DhtHandle, spawn};
use crate::server::test_net::{lan_host, node_on};

const RESPONDERS: usize = 24;
const AFTER_CHANGE: usize = 16;
/// Total patience for the whole "moved pings" phase (inside the 60 s budget of the test).
const LAN_PATIENCE: Duration = Duration::from_secs(30);

/// Pings until the first success; after a failure past `until` returns the last error.
/// Always makes at least one attempt.
async fn ping_patiently(
    dht: &DhtHandle,
    host: &str,
    port: u16,
    until: tokio::time::Instant,
) -> Result<(), String> {
    loop {
        match dht.ping(host, port).await {
            Ok(_) => return Ok(()),
            Err(err) if tokio::time::Instant::now() >= until => return Err(format!("{err:?}")),
            Err(_) => tokio::time::sleep(Duration::from_millis(250)).await,
        }
    }
}

/// Whether plain UDP sockets (no DHT) get an echo through the LAN address `lan`.
async fn lan_delivers_udp(lan: &str) -> bool {
    let lan = lan.to_string();
    tokio::task::spawn_blocking(move || {
        let server = std::net::UdpSocket::bind("0.0.0.0:0").expect("bind server");
        let client = std::net::UdpSocket::bind("0.0.0.0:0").expect("bind client");
        let timeout = Some(Duration::from_millis(500));
        server.set_read_timeout(timeout).expect("server timeout");
        client.set_read_timeout(timeout).expect("client timeout");
        let target = format!("{lan}:{}", server.local_addr().expect("server addr").port());
        for _ in 0..4 {
            let _ = client.send_to(&[1u8], &target);
            let mut buf = [0u8; 8];
            if let Ok((_, from)) = server.recv_from(&mut buf) {
                let _ = server.send_to(&[2u8], from);
                if client.recv_from(&mut buf).is_ok() {
                    return true;
                }
            }
            std::thread::sleep(Duration::from_secs(1));
        }
        false
    })
    .await
    .unwrap_or(false)
}

/// Ephemeral wildcard-bound nodes that answer `FIND_NODE`; returns their ports.
/// The join handles are kept so that the nodes live until the end of the test.
async fn responders(rt: &libudx::UdxRuntime) -> (Vec<tokio::task::JoinHandle<()>>, Vec<u16>) {
    let mut tasks = Vec::new();
    let mut ports = Vec::new();
    let mut handles: Vec<DhtHandle> = Vec::new();
    for _ in 0..RESPONDERS {
        let cfg = DhtConfig {
            bootstrap: vec![],
            host: "0.0.0.0".into(),
            port: 0,
            ephemeral: Some(true),
            ..DhtConfig::default()
        };
        let (task, handle) = spawn(rt, cfg).await.expect("spawn responder");
        ports.push(handle.local_port().await.expect("responder port"));
        handles.push(handle);
        tasks.push(tokio::spawn(async move {
            let _ = task.await;
        }));
    }
    // Keep the handles alive in a task that never finishes before the test ends.
    tasks.push(tokio::spawn(async move {
        let _handles = handles;
        std::future::pending::<()>().await;
    }));
    (tasks, ports)
}

/// Polls `remote_address()` of `handle` (every 50 ms, up to 5 s) until its host equals `host`.
async fn wait_host(handle: &crate::hyperdht::HyperDhtHandle, host: &str) -> Option<String> {
    let mut last = None;
    for _ in 0..100 {
        if let Ok(Some(address)) = handle.remote_address().await {
            if address.host == host {
                return None;
            }
            last = Some(address.host);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Some(format!("{last:?}"))
}

#[tokio::test]
async fn client_after_network_change_punches_to_server_at_old_address() {
    within(async {
        // given: a server on 127.0.0.1 like the cluster nodes, a wildcard-bound client
        // and 24 responders that the client can reach at 127.0.0.1 or at the LAN address
        let cl = cluster(3).await;
        let lan = lan_host();
        let mut server = serve(node(&cl, true).await, &key(1), ServerOpts::default()).await;
        let client = node_on(&cl, "0.0.0.0", true).await;
        let responder_rt = libudx::UdxRuntime::new().expect("udx runtime");
        let (_tasks, ports) = responders(&responder_rt).await;

        // home: every responder sees the client at 127.0.0.1 (the server's address)
        for port in &ports {
            client
                .handle
                .dht()
                .ping("127.0.0.1", *port)
                .await
                .expect("home ping");
        }
        if let Some(seen) = wait_host(&client.handle, "127.0.0.1").await {
            panic!("home: remote_address host never became 127.0.0.1, last {seen}");
        }

        // when: the network changes (sixteen responders now see the LAN address)
        let until = tokio::time::Instant::now() + LAN_PATIENCE;
        for port in ports.iter().take(AFTER_CHANGE) {
            if let Err(err) = ping_patiently(client.handle.dht(), &lan, *port, until).await {
                if !lan_delivers_udp(&lan).await {
                    eprintln!(
                        "SKIPPED (environment): plain UDP sockets do not reach {lan} either; \
                         the host drops UDP to its own LAN address"
                    );
                    return;
                }
                panic!(
                    "moved ping to {lan}:{port} failed for {LAN_PATIENCE:?} although plain UDP \
                     sockets reach {lan}: {err}"
                );
            }
        }
        if let Some(seen) = wait_host(&client.handle, &lan).await {
            panic!("change: remote_address host never became {lan}, last {seen}");
        }
        let conn = client
            .handle
            .connect_with_options(
                &key(2),
                key(1).public_key,
                &[],
                &client.rt,
                ConnectOpts::default(),
            )
            .await
            .expect("connects");

        // then: connected to the server, over a hole-punch socket and not the node socket
        assert_eq!(conn.remote_public_key, key(1).public_key);
        let local_port = conn.socket.local_addr().await.expect("local_addr").port();
        let node_port = primary_port(&client.handle).await;
        assert_ne!(
            local_port, node_port,
            "the connection left from the node socket (direct path), not a holepunch"
        );
        let accepted = tokio::time::timeout(Duration::from_secs(10), server.incoming.recv()).await;
        assert!(
            accepted.expect("no incoming connection in 10 s").is_some(),
            "incoming channel closed"
        );
    })
    .await;
}

#[tokio::test]
async fn ping_patiently_gives_up_at_the_deadline() {
    // given: a port where nobody listens
    let port = {
        let sock = std::net::UdpSocket::bind("127.0.0.1:0").expect("bind");
        sock.local_addr().expect("addr").port()
    };
    let rt = libudx::UdxRuntime::new().expect("udx runtime");
    let cfg = DhtConfig {
        bootstrap: vec![],
        host: "127.0.0.1".into(),
        port: 0,
        ephemeral: Some(true),
        ..DhtConfig::default()
    };
    let (_task, handle) = spawn(&rt, cfg).await.expect("spawn");
    // when: the deadline has already passed
    let result = ping_patiently(&handle, "127.0.0.1", port, tokio::time::Instant::now()).await;
    // then: exactly one attempt and a timeout error
    let err = result.expect_err("nobody answers");
    assert!(
        err.to_lowercase().contains("timeout"),
        "expected a timeout, got {err}"
    );
}

#[tokio::test]
async fn lan_delivers_udp_tells_loopback_from_a_dead_address() {
    assert!(lan_delivers_udp("127.0.0.1").await, "loopback must echo");
    // TEST-NET-1 (documentation range): nothing answers there.
    assert!(
        !lan_delivers_udp("192.0.2.1").await,
        "a dead address must not echo"
    );
}
