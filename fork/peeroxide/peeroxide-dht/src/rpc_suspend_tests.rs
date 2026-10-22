//! `suspend`/`resume` of a bare DHT node (no servers): what a suspended node does with
//! incoming and outgoing requests, and that it wakes up again on the same port.

use std::net::UdpSocket;
use std::time::Duration;

use libudx::UdxRuntime;
use tokio::time::timeout;

use crate::rpc::{DhtConfig, DhtError, DhtHandle, spawn};

const LOOPBACK: &str = "127.0.0.1";

fn config(bootstrap: Vec<String>) -> DhtConfig {
    DhtConfig {
        bootstrap,
        host: LOOPBACK.into(),
        firewalled: false,
        ephemeral: Some(false),
        ..DhtConfig::default()
    }
}

/// Two nodes on loopback: `b` bootstraps through `a`. Returns the runtime so it lives on.
async fn pair() -> (UdxRuntime, DhtHandle, DhtHandle) {
    let rt = UdxRuntime::new().expect("runtime");
    let (_task, a) = spawn(&rt, config(vec![])).await.expect("spawn a");
    let a_port = a.local_port().await.expect("port");
    let (_task, b) = spawn(&rt, config(vec![format!("{LOOPBACK}:{a_port}")]))
        .await
        .expect("spawn b");
    b.bootstrapped().await.expect("b bootstraps");
    (rt, a, b)
}

#[tokio::test]
async fn suspended_node_is_silent_and_resume_answers_on_the_same_port() {
    let (_rt, a, b) = pair().await;
    let port = b.local_port().await.expect("port");
    a.ping(LOOPBACK, port).await.expect("ping before suspend");

    b.suspend().await.expect("suspend");
    assert!(b.is_suspended());
    let silent = timeout(Duration::from_secs(30), a.ping(LOOPBACK, port))
        .await
        .expect("the ping ends");
    assert!(
        silent.is_err(),
        "a suspended node must not answer: {silent:?}"
    );

    b.resume().await.expect("resume");
    assert!(!b.is_suspended());
    assert_eq!(b.local_port().await.expect("port"), port, "same port");
    a.ping(LOOPBACK, port).await.expect("ping after resume");
}

#[tokio::test]
async fn suspended_node_refuses_to_send() {
    let (_rt, a, b) = pair().await;
    let a_port = a.local_port().await.expect("port");
    b.suspend().await.expect("suspend");
    let sent = b.ping(LOOPBACK, a_port).await;
    assert!(matches!(sent, Err(DhtError::Suspended)), "got {sent:?}");
    b.resume().await.expect("resume");
    b.ping(LOOPBACK, a_port).await.expect("sends again");
}

#[tokio::test]
async fn request_in_flight_fails_with_suspended() {
    let (_rt, _a, b) = pair().await;
    let hole = UdpSocket::bind((LOOPBACK, 0)).expect("black hole");
    let hole_port = hole.local_addr().expect("addr").port();
    let waiting = {
        let b = b.clone();
        tokio::spawn(async move { b.ping(LOOPBACK, hole_port).await })
    };
    tokio::time::sleep(Duration::from_millis(200)).await;
    b.suspend().await.expect("suspend");
    let ended = timeout(Duration::from_secs(1), waiting)
        .await
        .expect("fails at once, not by timeout")
        .expect("task");
    assert!(matches!(ended, Err(DhtError::Suspended)), "got {ended:?}");
}

#[tokio::test]
async fn suspend_and_resume_are_idempotent() {
    let (_rt, _a, b) = pair().await;
    b.resume().await.expect("resume while running is harmless");
    assert!(!b.is_suspended());
    b.suspend().await.expect("suspend");
    b.suspend().await.expect("suspend again");
    assert!(b.is_suspended());
    b.resume().await.expect("resume");
    b.resume().await.expect("resume again");
    assert!(!b.is_suspended());
}
