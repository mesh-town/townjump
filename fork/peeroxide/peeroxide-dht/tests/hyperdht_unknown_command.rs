//! Unknown commands: unknown commands get error 1 (UNKNOWN_COMMAND); PLUGIN (10) gets no reply.
//! Reference: testvectors/deltas/plugin.json (`withId`).
use libudx::UdxRuntime;
use peeroxide_dht::hyperdht::{HyperDhtConfig, spawn};
use peeroxide_dht::hyperdht_messages::PLUGIN;
use peeroxide_dht::rpc::{DhtConfig, UserRequestParams};

async fn node(rt: &UdxRuntime, firewalled: bool) -> peeroxide_dht::hyperdht::HyperDhtHandle {
    let mut dht = DhtConfig::default();
    dht.bootstrap = vec![];
    dht.host = "127.0.0.1".to_string();
    dht.firewalled = firewalled;
    let mut cfg = HyperDhtConfig::default();
    cfg.dht = dht;
    let (_join, handle, _rx) = spawn(rt, cfg).await.expect("spawn");
    handle
}

fn params(command: u64, timeout_ms: u64) -> UserRequestParams {
    let mut p = UserRequestParams::default();
    p.command = command;
    p.target = Some([7u8; 32]);
    p.timeout_ms = Some(timeout_ms);
    p.retries = Some(0);
    p
}

#[tokio::test]
async fn unknown_command_gets_error_1() {
    let rt = UdxRuntime::new().unwrap();
    let server = node(&rt, false).await;
    let client = node(&rt, true).await;
    let port = server.dht().local_port().await.unwrap();
    let r = client
        .dht()
        .request(params(99, 2000), "127.0.0.1", port)
        .await;
    assert_eq!(r.expect("reply").error, 1);
}

#[tokio::test]
async fn plugin_gets_no_reply() {
    let rt = UdxRuntime::new().unwrap();
    let server = node(&rt, false).await;
    let client = node(&rt, true).await;
    let port = server.dht().local_port().await.unwrap();
    let r = client
        .dht()
        .request(params(PLUGIN, 700), "127.0.0.1", port)
        .await;
    assert!(r.is_err(), "PLUGIN must time out, got {r:?}");
}
