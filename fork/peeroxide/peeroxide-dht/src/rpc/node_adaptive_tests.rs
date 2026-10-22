// JS: dht-rpc/index.js (_updateNetworkState)
//! Adaptive mode on a real `DhtNode` (sockets bound, loop not running): countdown, check round,
//! outcomes, wake-up and `set_adaptive`. Own config instead of `node_test_support::setup`,
//! which turns the mode off (empty bootstrap, `ephemeral: Some(true)`).

use std::time::Duration;

use libudx::UdxRuntime;

use super::super::node::{DhtNode, build_node};
use super::super::node_test_support::{PORT_B, node_b, spot};
use super::super::requests::StandaloneRequest;
use super::super::{DhtConfig, DhtHandle, DhtMode};
use crate::hyperdht_messages::FIREWALL_OPEN;
use crate::io::{IoEvent, ResolvedRequest, TimeoutEvent};
use crate::messages::Ipv4Peer;
use crate::peer::peer_id;

const NAT_HOST: &str = "198.51.100.7";
const BOOTSTRAP_PORT: u16 = 9050;

struct Rig {
    node: DhtNode,
    handle: DhtHandle,
    _rt: UdxRuntime,
}

fn config(ephemeral: Option<bool>, firewalled: bool) -> DhtConfig {
    DhtConfig {
        bootstrap: vec![format!("127.0.0.1:{BOOTSTRAP_PORT}")],
        ephemeral,
        firewalled,
        stable_ticks: 2,
        ..DhtConfig::default()
    }
}

async fn rig(config: DhtConfig) -> Rig {
    let rt = UdxRuntime::new().expect("runtime");
    let (mut node, handle) = build_node(&rt, config).await.expect("build_node");
    node.bootstrapped = true;
    Rig {
        node,
        handle,
        _rt: rt,
    }
}

impl Rig {
    fn mode(&self) -> DhtMode {
        self.handle.mode.borrow().clone()
    }

    fn table_id(&self) -> [u8; 32] {
        *self.node.table.lock().expect("table").id()
    }

    /// Three responders told the node it is seen as `host:local_port`.
    fn seed_window(&mut self, host: &str) {
        let to = Ipv4Peer {
            host: host.into(),
            port: self.node.local_port,
        };
        (0..3).for_each(|i| {
            self.node.node_nat.sample(&to, &spot(20_000 + i), 0);
        });
    }

    fn check_tids(&self) -> Vec<u16> {
        let mut tids: Vec<u16> = (self.node.standalone_tids.iter())
            .filter(|(_, r)| matches!(r, StandaloneRequest::NatCheck))
            .map(|(tid, _)| *tid)
            .collect();
        tids.sort_unstable();
        tids
    }

    /// A firewalled node with a table node and a bootstrap node, check round started.
    async fn checking() -> Rig {
        let mut r = rig(config(None, true)).await;
        r.node.table.lock().expect("table").add(node_b());
        r.seed_window(NAT_HOST);
        r.node.update_network_state();
        r
    }

    fn answer(&mut self, tid: u16, server_arrival: bool) {
        let to = Ipv4Peer {
            host: NAT_HOST.into(),
            port: self.node.local_port,
        };
        self.node.handle_io_event(reply(tid, server_arrival, to));
    }

    fn expire(&mut self, tid: u16) {
        self.node.handle_timeout(TimeoutEvent {
            tid,
            to: spot(PORT_B),
            command: 0,
            internal: true,
            target: None,
            node_id: None,
        });
    }

    fn promote_by_check(&mut self) {
        for tid in self.check_tids() {
            self.answer(tid, true);
        }
    }
}

fn reply(tid: u16, server_arrival: bool, to: Ipv4Peer) -> IoEvent {
    IoEvent::Response {
        server_arrival,
        tid,
        from: spot(PORT_B),
        to,
        id: None,
        token: None,
        closer_nodes: vec![],
        error: 0,
        value: None,
        rtt: Duration::from_millis(5),
        request: ResolvedRequest {
            tid,
            to: spot(PORT_B),
            command: 0,
            internal: true,
            target: None,
        },
    }
}

// ── start mode ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn start_mode_is_ephemeral_adaptive_with_bootstrap() {
    let r = rig(config(None, false)).await;
    let mode = r.mode();
    assert!(mode.ephemeral && mode.adaptive && mode.address.is_none());
}

#[tokio::test]
async fn start_mode_some_false_is_persistent_without_adaptive() {
    let r = rig(config(Some(false), false)).await;
    let mode = r.mode();
    assert!(!mode.ephemeral && !mode.adaptive);
}

#[tokio::test]
async fn start_mode_empty_bootstrap_is_persistent() {
    let cfg = DhtConfig {
        bootstrap: vec![],
        ..config(None, false)
    };
    let r = rig(cfg).await;
    let mode = r.mode();
    assert!(!mode.ephemeral && !mode.adaptive);
}

// ── promotion ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn open_path_promotes_after_stable_ticks() {
    let mut r = rig(config(None, false)).await;
    r.seed_window(NAT_HOST);
    r.node.adaptive_tick();
    assert!(r.mode().ephemeral, "countdown not over after one tick");
    r.node.adaptive_tick();
    let address = Ipv4Peer {
        host: NAT_HOST.into(),
        port: r.node.local_port,
    };
    let mode = r.mode();
    assert!(!mode.ephemeral);
    assert_eq!(mode.address, Some(address.clone()));
    assert_eq!(r.table_id(), peer_id(&address.host, address.port));
}

#[tokio::test]
async fn firewalled_node_starts_one_check_round() {
    let mut r = Rig::checking().await;
    let first = r.check_tids();
    assert_eq!(first.len(), 2, "one table node plus one bootstrap node");
    assert!(r.node.nat_check.is_some());
    r.node.update_network_state();
    assert_eq!(r.check_tids(), first, "no second round while one runs");
}

#[tokio::test]
async fn check_outcome_promotes_and_opens() {
    let mut r = Rig::checking().await;
    r.promote_by_check();
    let mode = r.mode();
    assert!(!mode.ephemeral && !mode.firewalled);
    assert_eq!(mode.address.map(|a| a.host), Some(NAT_HOST.to_string()));
    assert!(r.node.nat_check.is_none());
}

#[tokio::test]
async fn one_reply_and_one_timeout_still_promote() {
    let mut r = Rig::checking().await;
    let tids = r.check_tids();
    r.answer(tids[0], true);
    assert!(r.mode().ephemeral, "round is not over yet");
    r.expire(tids[1]);
    assert!(!r.mode().ephemeral);
}

#[tokio::test]
async fn client_socket_replies_keep_ephemeral() {
    let mut r = Rig::checking().await;
    for tid in r.check_tids() {
        r.answer(tid, false);
    }
    let mode = r.mode();
    assert!(mode.ephemeral && mode.firewalled);
    assert!(r.node.nat_check.is_none());
}

#[tokio::test]
async fn host_moved_during_check_keeps_ephemeral() {
    let mut r = Rig::checking().await;
    let mut moved = super::super::nat_window::NatWindow::new();
    (0..3).for_each(|_| {
        moved.add("203.0.113.9", r.node.local_port);
    });
    r.node.node_nat.replace_window(moved);
    r.promote_by_check();
    let mode = r.mode();
    assert!(mode.ephemeral && mode.firewalled);
}

#[tokio::test]
async fn timeouts_end_the_round_without_promotion() {
    let mut r = Rig::checking().await;
    for tid in r.check_tids() {
        r.expire(tid);
    }
    assert!(r.node.nat_check.is_none());
    assert!(r.mode().ephemeral);
}

#[tokio::test]
async fn suspend_clears_the_check() {
    let mut r = Rig::checking().await;
    r.node.suspend_node();
    assert!(r.node.nat_check.is_none());
    assert!(r.check_tids().is_empty());
}

// ── wake-up and set_adaptive ──────────────────────────────────────────────────

#[tokio::test]
async fn wakeup_demotes_and_keeps_table_id() {
    let mut r = rig(config(None, false)).await;
    r.seed_window(NAT_HOST);
    (0..2).for_each(|_| r.node.adaptive_tick());
    let id = r.table_id();
    r.node.wake_up();
    let mode = r.mode();
    assert!(mode.ephemeral && mode.address.is_none());
    assert_eq!(r.table_id(), id);
    assert!(!mode.firewalled, "configured open stays open");
}

#[tokio::test]
async fn wakeup_restores_firewalled() {
    let mut r = Rig::checking().await;
    r.promote_by_check();
    assert!(!r.mode().firewalled);
    r.node.wake_up();
    let mode = r.mode();
    assert!(mode.ephemeral && mode.firewalled);
    assert!(r.node.io.is_firewalled());
    assert_ne!(r.node.node_nat.remote_state().1, FIREWALL_OPEN);
}

#[tokio::test]
async fn set_adaptive_false_demotes_but_keeps_firewalled_state() {
    let mut r = Rig::checking().await;
    r.promote_by_check();
    r.node.set_adaptive(false);
    let mode = r.mode();
    assert!(mode.ephemeral && !mode.adaptive);
    assert!(!mode.firewalled, "the network did not change");
}

#[tokio::test]
async fn set_adaptive_false_on_configured_persistent_is_noop() {
    let mut r = rig(config(Some(false), false)).await;
    r.node.set_adaptive(false);
    assert!(!r.mode().ephemeral);
}

#[tokio::test]
async fn set_adaptive_true_restarts_the_countdown() {
    let mut r = rig(config(None, false)).await;
    r.seed_window(NAT_HOST);
    r.node.adaptive_tick();
    r.node.set_adaptive(true);
    r.node.adaptive_tick();
    assert!(r.mode().ephemeral, "countdown started over");
    r.node.adaptive_tick();
    assert!(!r.mode().ephemeral);
}
