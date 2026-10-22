//! Unit tests of `hyperdht`: config, `KeyPair`, compact uint, errors, `spawn`, stream helpers, bootstrap.
//! Neighbours: `hyperdht_topology_tests` (connect topology), `hyperdht_passive_tests`, `hyperdht_mutable_latest_tests`.

use super::records::encode_compact_uint;
use super::stream::next_stream_id;
use super::*;
use crate::crypto::{sign_detached, verify_detached};
use crate::hyperdht_messages::UdxInfo;
use crate::messages::Ipv4Peer;
use crate::noise_wrap::NoiseWrapResult;
use crate::persistent::PersistentConfig;
use crate::rpc::DhtConfig;

#[test]
fn hyperdht_config_defaults() {
    let cfg = HyperDhtConfig::default();
    assert_eq!(cfg.dht.port, 0);
    assert_eq!(cfg.dht.host, "0.0.0.0");
    assert_eq!(cfg.dht.concurrency, 10);
    assert!(cfg.dht.bootstrap.is_empty());
    assert_eq!(
        cfg.persistent.max_records,
        PersistentConfig::default().max_records
    );
    assert_eq!(
        cfg.persistent.max_per_key,
        PersistentConfig::default().max_per_key
    );
}

#[test]
fn keypair_generate_produces_unique_keys() {
    let kp1 = KeyPair::generate();
    let kp2 = KeyPair::generate();
    assert_ne!(kp1.public_key, kp2.public_key);
}

#[test]
fn keypair_from_seed_deterministic() {
    let seed = [0x42u8; 32];
    let kp1 = KeyPair::from_seed(seed);
    let kp2 = KeyPair::from_seed(seed);
    assert_eq!(kp1.public_key, kp2.public_key);
    assert_eq!(kp1.secret_key, kp2.secret_key);
}

#[test]
fn keypair_public_key_matches_secret() {
    let kp = KeyPair::from_seed([0x11u8; 32]);
    assert_eq!(&kp.secret_key[32..], &kp.public_key);
}

#[test]
fn keypair_sign_verify_roundtrip() {
    let kp = KeyPair::generate();
    let msg = b"test message";
    let sig = sign_detached(msg, &kp.secret_key);
    assert!(verify_detached(&sig, msg, &kp.public_key));
}

#[test]
fn encode_compact_uint_round_trips() {
    use crate::compact_encoding::{State, decode_uint};
    for val in [0u64, 1, 127, 128, 255, 65535, (1 << 53) - 1] {
        let bytes = encode_compact_uint(val);
        let mut s = State::from_buffer(&bytes);
        let decoded = decode_uint(&mut s).unwrap();
        assert_eq!(decoded, val, "compact uint round-trip failed for {val}");
    }
}

#[test]
fn hyperdht_error_display() {
    let e = HyperDhtError::Destroyed;
    assert!(e.to_string().contains("destroyed"));
    let e2 = HyperDhtError::InvalidSignature;
    assert!(e2.to_string().contains("signature"));
}

#[test]
fn keypair_debug_hides_secret() {
    let kp = KeyPair::from_seed([0x42u8; 32]);
    let dbg = format!("{kp:?}");
    assert!(dbg.contains("KeyPair"));
    assert!(!dbg.contains("secret_key"));
}

#[tokio::test]
async fn spawn_and_destroy() {
    let runtime = libudx::UdxRuntime::new().expect("runtime");
    let config = HyperDhtConfig {
        dht: DhtConfig {
            bootstrap: vec![],
            port: 0,
            ..DhtConfig::default()
        },
        persistent: PersistentConfig::default(),
    };
    let (join, handle, _server_rx) = spawn(&runtime, config).await.expect("spawn");
    handle.destroy().await.expect("destroy");
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), join).await;
}

#[tokio::test]
async fn wire_stats_starts_at_zero_and_is_addressable() {
    let runtime = libudx::UdxRuntime::new().expect("runtime");
    let config = HyperDhtConfig {
        dht: DhtConfig {
            bootstrap: vec![],
            port: 0,
            ..DhtConfig::default()
        },
        persistent: PersistentConfig::default(),
    };
    let (join, handle, _rx) = spawn(&runtime, config).await.expect("spawn");
    let (sent, received) = handle.wire_stats();
    assert_eq!(sent, 0, "no traffic yet");
    assert_eq!(received, 0);
    // Counters are shared via Arc — incrementing through `wire_counters()`
    // must be visible via `wire_stats()`.
    let counters = handle.wire_counters();
    counters
        .bytes_sent
        .fetch_add(123, std::sync::atomic::Ordering::Relaxed);
    counters
        .bytes_received
        .fetch_add(456, std::sync::atomic::Ordering::Relaxed);
    let (sent, received) = handle.wire_stats();
    assert_eq!(sent, 123);
    assert_eq!(received, 456);
    handle.destroy().await.expect("destroy");
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), join).await;
}

#[test]
fn next_stream_id_is_unique() {
    let a = next_stream_id();
    let b = next_stream_id();
    let c = next_stream_id();
    assert_ne!(a, b);
    assert_ne!(b, c);
    assert_ne!(a, c);
}

#[tokio::test]
async fn establish_stream_missing_udx_info() {
    let runtime = libudx::UdxRuntime::new().expect("runtime");
    let nw_result = NoiseWrapResult {
        remote_public_key: [0xAA; 32],
        tx: [1; 32],
        rx: [2; 32],
        handshake_hash: [3; 64],
        holepunch_secret: [4; 32],
        is_initiator: true,
    };
    let result = ConnectResult {
        remote_public_key: [0xAA; 32],
        server_address: Ipv4Peer {
            host: "127.0.0.1".into(),
            port: 9999,
        },
        client_address: Ipv4Peer {
            host: "127.0.0.1".into(),
            port: 8888,
        },
        is_relayed: false,
        noise: nw_result,
        local_stream_id: 1,
        remote_udx: None,
        udx_socket: None,
    };
    let err = establish_stream(&result, &runtime).await.unwrap_err();
    assert!(matches!(err, HyperDhtError::StreamEstablishment(_)));
}

#[tokio::test]
async fn establish_stream_bad_address() {
    let runtime = libudx::UdxRuntime::new().expect("runtime");
    let nw_result = NoiseWrapResult {
        remote_public_key: [0xBB; 32],
        tx: [1; 32],
        rx: [2; 32],
        handshake_hash: [3; 64],
        holepunch_secret: [4; 32],
        is_initiator: true,
    };
    let result = ConnectResult {
        remote_public_key: [0xBB; 32],
        server_address: Ipv4Peer {
            host: "not-an-ip".into(),
            port: 9999,
        },
        client_address: Ipv4Peer {
            host: "127.0.0.1".into(),
            port: 8888,
        },
        is_relayed: false,
        noise: nw_result,
        local_stream_id: next_stream_id(),
        remote_udx: Some(UdxInfo {
            version: 1,
            reusable_socket: true,
            id: 42,
            seq: 0,
        }),
        udx_socket: None,
    };
    let err = establish_stream(&result, &runtime).await.unwrap_err();
    assert!(matches!(err, HyperDhtError::StreamEstablishment(_)));
}

#[tokio::test]
async fn establish_stream_remote_id_overflow() {
    let runtime = libudx::UdxRuntime::new().expect("runtime");
    let nw_result = NoiseWrapResult {
        remote_public_key: [0xCC; 32],
        tx: [1; 32],
        rx: [2; 32],
        handshake_hash: [3; 64],
        holepunch_secret: [4; 32],
        is_initiator: true,
    };
    let result = ConnectResult {
        remote_public_key: [0xCC; 32],
        server_address: Ipv4Peer {
            host: "127.0.0.1".into(),
            port: 9999,
        },
        client_address: Ipv4Peer {
            host: "127.0.0.1".into(),
            port: 8888,
        },
        is_relayed: false,
        noise: nw_result,
        local_stream_id: next_stream_id(),
        remote_udx: Some(UdxInfo {
            version: 1,
            reusable_socket: true,
            id: u64::from(u32::MAX) + 1,
            seq: 0,
        }),
        udx_socket: None,
    };
    let err = establish_stream(&result, &runtime).await.unwrap_err();
    assert!(matches!(err, HyperDhtError::StreamEstablishment(_)));
}

#[test]
fn default_bootstrap_has_three_nodes() {
    assert_eq!(DEFAULT_BOOTSTRAP.len(), 3);
    for entry in &DEFAULT_BOOTSTRAP {
        assert!(entry.contains('@'), "missing @ in {entry}");
        assert!(entry.ends_with(":49737"), "wrong port in {entry}");
    }
}

#[test]
fn with_public_bootstrap_populates_nodes() {
    let cfg = HyperDhtConfig::with_public_bootstrap();
    assert_eq!(cfg.dht.bootstrap.len(), 3);
    assert_eq!(cfg.dht.bootstrap[0], DEFAULT_BOOTSTRAP[0]);
    assert_eq!(cfg.dht.bootstrap[1], DEFAULT_BOOTSTRAP[1]);
    assert_eq!(cfg.dht.bootstrap[2], DEFAULT_BOOTSTRAP[2]);
    assert_eq!(cfg.dht.port, 0);
    assert!(cfg.dht.firewalled);
}

#[tokio::test]
async fn current_relay_addresses_starts_empty() {
    use libudx::UdxRuntime;

    let runtime = UdxRuntime::new().expect("runtime");
    let cfg = HyperDhtConfig {
        dht: crate::rpc::DhtConfig {
            bootstrap: vec![],
            ephemeral: Some(true),
            ..crate::rpc::DhtConfig::default()
        },
        ..HyperDhtConfig::default()
    };
    let (_task, handle, _rx) = spawn(&runtime, cfg).await.expect("spawn");

    assert!(
        handle.current_relay_addresses().is_empty(),
        "current_relay_addresses should be empty before any announce()"
    );

    let _ = handle.destroy().await;
}

#[tokio::test]
async fn current_relay_addresses_is_shared_across_clones() {
    use libudx::UdxRuntime;

    let runtime = UdxRuntime::new().expect("runtime");
    let cfg = HyperDhtConfig {
        dht: crate::rpc::DhtConfig {
            bootstrap: vec![],
            ephemeral: Some(true),
            ..crate::rpc::DhtConfig::default()
        },
        ..HyperDhtConfig::default()
    };
    let (_task, handle, _rx) = spawn(&runtime, cfg).await.expect("spawn");

    let clone = handle.clone();

    {
        let mut guard = handle
            .current_relay_addresses
            .lock()
            .expect("lock current_relay_addresses");
        *guard = vec![Ipv4Peer {
            host: "1.2.3.4".to_string(),
            port: 49737,
        }];
    }

    let via_clone = clone.current_relay_addresses();
    assert_eq!(via_clone.len(), 1);
    assert_eq!(via_clone[0].host, "1.2.3.4");
    assert_eq!(via_clone[0].port, 49737);

    let _ = handle.destroy().await;
}
