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
        for port in ports.iter().take(AFTER_CHANGE) {
            client
                .handle
                .dht()
                .ping(&lan, *port)
                .await
                .expect("moved ping");
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
