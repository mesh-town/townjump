use super::consts::{DOWN_HINTS_RATE_LIMIT, OLD_NODE, RECENT_NODE, REFRESH_TICKS};
use super::*;
use crate::query::parse_bootstrap_str;

#[test]
fn test_dht_config_defaults() {
    let cfg = DhtConfig::default();
    assert_eq!(cfg.port, 0);
    assert_eq!(cfg.host, "0.0.0.0");
    assert_eq!(cfg.concurrency, 10);
    assert_eq!(cfg.max_window, 80);
    assert!(cfg.firewalled);
    assert!(cfg.bootstrap.is_empty());
    assert!(cfg.ephemeral.is_none());
}

#[test]
fn test_bootstrap_parse_simple() {
    let (host, port) = parse_bootstrap_str("127.0.0.1:10001").expect("parse");
    assert_eq!(host, "127.0.0.1");
    assert_eq!(port, 10001);
}

#[test]
fn test_bootstrap_parse_with_at() {
    let (host, port) =
        parse_bootstrap_str("10.0.0.1@bootstrap.example.com:24242").expect("should parse");
    assert_eq!(host, "10.0.0.1");
    assert_eq!(port, 24242);
}

#[test]
fn test_bootstrap_parse_bad_no_colon() {
    assert!(parse_bootstrap_str("localhost").is_none());
}

#[test]
fn test_bootstrap_parse_bad_port() {
    assert!(parse_bootstrap_str("localhost:notaport").is_none());
}

#[test]
fn test_err_display() {
    let e = DhtError::Destroyed;
    assert!(e.to_string().contains("destroyed"));
}

#[test]
fn test_down_hints_rate_limit_constant() {
    assert_eq!(DOWN_HINTS_RATE_LIMIT, 50);
}

#[test]
fn test_tick_constants() {
    assert_eq!(REFRESH_TICKS, 60);
    assert_eq!(RECENT_NODE, 12);
    assert_eq!(OLD_NODE, 360);
}

#[tokio::test]
async fn remote_address_starts_unsettled() {
    use libudx::UdxRuntime;

    let runtime = UdxRuntime::new().expect("runtime");
    let cfg = DhtConfig {
        bootstrap: vec![],
        ephemeral: Some(true),
        ..DhtConfig::default()
    };
    let (_task, handle) = spawn(&runtime, cfg).await.expect("spawn");

    let addr = handle.remote_address().await.expect("remote_address");
    assert!(
        addr.is_none(),
        "remote_address should be None before any DHT responses arrive"
    );

    let firewall = handle.firewalled().await.expect("firewalled");
    assert_eq!(
        firewall,
        crate::hyperdht_messages::FIREWALL_UNKNOWN,
        "firewalled flag should start as UNKNOWN for a firewalled node"
    );

    let _ = handle.destroy().await;
}

#[tokio::test]
async fn remote_address_unfirewalled_starts_open() {
    use libudx::UdxRuntime;

    let runtime = UdxRuntime::new().expect("runtime");
    let cfg = DhtConfig {
        bootstrap: vec![],
        firewalled: false,
        ephemeral: Some(true),
        ..DhtConfig::default()
    };
    let (_task, handle) = spawn(&runtime, cfg).await.expect("spawn");

    let firewall = handle.firewalled().await.expect("firewalled");
    assert_eq!(
        firewall,
        crate::hyperdht_messages::FIREWALL_OPEN,
        "unfirewalled node should report FIREWALL_OPEN immediately"
    );

    let _ = handle.destroy().await;
}
