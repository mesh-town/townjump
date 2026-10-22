//! Tests for [`crate::nat_sampler`].

use std::time::Duration;

use tokio::sync::mpsc;

use super::*;
use crate::rpc::{DhtConfig, DhtHandle};
use crate::socket_pool::SocketPool;

async fn loopback_dht(
    runtime: &libudx::UdxRuntime,
    firewalled: bool,
) -> (
    tokio::task::JoinHandle<Result<(), crate::rpc::DhtError>>,
    DhtHandle,
) {
    let cfg = DhtConfig {
        bootstrap: vec![],
        host: "127.0.0.1".to_string(),
        ephemeral: Some(true),
        firewalled,
        ..DhtConfig::default()
    };
    crate::rpc::spawn(runtime, cfg).await.expect("spawn dht")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropping_receiver_stops_sampler_without_pongs() {
    let runtime = libudx::UdxRuntime::new().expect("runtime");
    let (_pt, primary) = loopback_dht(&runtime, true).await;
    // Black hole: never answers, so without the `closed()` branch the task keeps running until the ping fails (~1 s).
    let hole = tokio::net::UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("hole");
    primary.insert_node_for_test(crate::routing_table::Node {
        id: rand::random(),
        host: "127.0.0.1".to_string(),
        port: hole.local_addr().unwrap().port(),
        token: None,
        added_tick: 0,
        seen_tick: 0,
        pinged_tick: 0,
        down_hints: 0,
    });
    let pool = SocketPool::new("127.0.0.1".to_string());
    let socket_ref = pool.acquire(&runtime).await.expect("acquire");

    let metrics = tokio::runtime::Handle::current().metrics();
    let base = metrics.num_alive_tasks();
    let rx = spawn_sampler(&primary, socket_ref.socket.clone(), 4);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        metrics.num_alive_tasks() > base,
        "sampler task must be running while the ping waits"
    );
    drop(rx);

    let deadline = tokio::time::Instant::now() + Duration::from_millis(400);
    while metrics.num_alive_tasks() > base && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        metrics.num_alive_tasks() <= base,
        "sampler and its pings must end within 400 ms of dropping the receiver"
    );
    drop(hole);
    let _ = primary.destroy().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn forwarder_keeps_reply_lane_open() {
    let runtime = libudx::UdxRuntime::new().expect("runtime");
    let (_task, primary) = loopback_dht(&runtime, true).await;
    let (tx, rx) = mpsc::unbounded_channel::<libudx::Datagram>();
    let _forwarder = spawn_dht_reply_forwarder(primary.clone(), rx);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!tx.is_closed(), "forwarder must keep the reply lane open");
    let _ = primary.destroy().await;
}
