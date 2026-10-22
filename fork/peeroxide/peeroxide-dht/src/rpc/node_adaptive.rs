// JS: dht-rpc/index.js (_updateNetworkState, _onwakeup, PING_NAT)
//! `impl DhtNode`: adaptive mode. An ephemeral node counts ticks, checks that its sampled
//! address is reachable (open socket, or PING_NAT replies on the server socket) and then
//! becomes persistent; a wake-up after a pause drops it back to ephemeral.
//! Neighbours: `adaptive` (countdown), `nat_check` (one round), `node_nat` (window).

use crate::io::{IncomingRequest, RequestParams};
use crate::messages::Ipv4Peer;
use crate::peer::peer_id;

use super::adaptive::Adaptive;
use super::consts::CMD_PING_NAT;
use super::nat_check::{NatCheck, check_targets, parse_bootstrap};
use super::nat_window::NatWindow;
use super::node::DhtNode;
use super::requests::StandaloneRequest;
use super::types::{DhtConfig, DhtMode};

/// A node joins a network when it has bootstrap nodes or seed nodes to start from.
pub(super) fn joins_network(config: &DhtConfig) -> bool {
    !config.bootstrap.is_empty() || !config.nodes.is_empty()
}

/// Adaptive mode applies only when the caller left the mode open and there is a network to join.
pub(super) fn adaptive_on(config: &DhtConfig) -> bool {
    config.adaptive && config.ephemeral.is_none() && joins_network(config)
}

/// The address a persistent node's table id was derived from at build time.
pub(super) fn node_address(
    config: &DhtConfig,
    ephemeral: bool,
    local_port: u16,
) -> Option<Ipv4Peer> {
    let wildcard = config.host == "0.0.0.0" || config.host == "::";
    (!ephemeral && !wildcard).then(|| Ipv4Peer {
        host: config.host.clone(),
        port: local_port,
    })
}

pub(super) fn start_mode(
    adaptive: &Adaptive,
    config: &DhtConfig,
    ephemeral: bool,
    address: &Option<Ipv4Peer>,
) -> DhtMode {
    DhtMode {
        ephemeral,
        adaptive: adaptive.is_on(),
        firewalled: config.firewalled,
        address: address.clone(),
    }
}

impl DhtNode {
    /// Called once per tick after bootstrap; runs the check when the countdown says so.
    pub(super) fn adaptive_tick(&mut self) {
        if self
            .adaptive
            .on_tick(self.io.ephemeral, self.node_nat.host())
        {
            self.update_network_state();
        }
    }

    pub(super) fn update_network_state(&mut self) {
        if !self.io.ephemeral || self.nat_check.is_some() {
            return;
        }
        self.adaptive.begin_check(self.node_nat.host());
        let Some(snapshot) = self.node_nat.address() else {
            return;
        };
        if !self.io.is_firewalled() {
            return self.promote(snapshot.clone(), snapshot, None);
        }
        self.start_nat_check(snapshot);
    }

    /// Asks the most recently seen nodes (topped up with bootstrap) to reply to our server port.
    pub(super) fn start_nat_check(&mut self, snapshot: Ipv4Peer) {
        let nodes = self.table_peers();
        let bootstrap = parse_bootstrap(&self.config.bootstrap);
        let mut check = NatCheck::new(snapshot);
        for to in check_targets(nodes, &bootstrap) {
            let params = RequestParams {
                to,
                internal: true,
                command: CMD_PING_NAT,
                value: Some(self.local_port.to_le_bytes().to_vec()),
                ..Default::default()
            };
            let Some(tid) = self.io.create_request(params) else {
                continue;
            };
            self.standalone_tids
                .insert(tid, StandaloneRequest::NatCheck);
            check.track();
        }
        let done = check.is_done();
        self.nat_check = Some(check);
        if done {
            self.finish_nat_check();
        }
    }

    fn table_peers(&self) -> Vec<(u64, Ipv4Peer)> {
        let Ok(table) = self.table.lock() else {
            return Vec::new();
        };
        let own = *table.id();
        let peer = |n: &crate::routing_table::Node| {
            let peer = Ipv4Peer {
                host: n.host.clone(),
                port: n.port,
            };
            (n.seen_tick, peer)
        };
        table
            .closest(&own, table.len())
            .into_iter()
            .map(peer)
            .collect()
    }

    /// True when `tid` was a check probe: the reply is consumed here.
    pub(super) fn take_nat_check_reply(
        &mut self,
        tid: u16,
        server_arrival: bool,
        to: &Ipv4Peer,
    ) -> bool {
        if !matches!(
            self.standalone_tids.get(&tid),
            Some(StandaloneRequest::NatCheck)
        ) {
            return false;
        }
        self.standalone_tids.remove(&tid);
        let Some(check) = self.nat_check.as_mut() else {
            return true;
        };
        check.on_reply(server_arrival, to);
        if check.is_done() {
            self.finish_nat_check();
        }
        true
    }

    pub(super) fn on_nat_check_timeout(&mut self) {
        let Some(check) = self.nat_check.as_mut() else {
            return;
        };
        check.on_timeout();
        if check.is_done() {
            self.finish_nat_check();
        }
    }

    /// No outcome means the node stays ephemeral until its host changes (no retry, like JS).
    pub(super) fn finish_nat_check(&mut self) {
        let Some(check) = self.nat_check.take() else {
            return;
        };
        let Some(reachable) = check.outcome(self.node_nat.host(), self.local_port) else {
            return;
        };
        self.io.set_firewalled(false);
        self.node_nat.open(self.local_port);
        self.publish_mode();
        let snapshot = check.snapshot().clone();
        self.promote(snapshot, reachable, Some(check.into_window()));
    }

    /// Becomes persistent if nothing changed since `snapshot` was taken.
    pub(super) fn promote(
        &mut self,
        snapshot: Ipv4Peer,
        reachable: Ipv4Peer,
        window: Option<NatWindow>,
    ) {
        let unchanged = self.node_nat.address().as_ref() == Some(&snapshot);
        let usable = reachable.host == snapshot.host && reachable.port != 0;
        if !(self.io.ephemeral && self.adaptive.is_on() && unchanged && usable) {
            return;
        }
        self.io.ephemeral = false;
        if let Some(window) = window {
            self.node_nat.replace_window(window);
        }
        if let Ok(mut table) = self.table.lock() {
            table.rebuild_with_id(peer_id(&reachable.host, reachable.port));
        }
        self.node_address = Some(reachable);
        if self.bootstrapped {
            self.run_refresh();
        }
        self.publish_mode();
    }

    /// Back to ephemeral. The table id stays (JS keeps it too).
    pub(super) fn demote(&mut self) {
        self.io.ephemeral = true;
        self.node_address = None;
        self.publish_mode();
    }

    /// After a wake-up the network may have changed: forget that the port was proven open,
    /// so the next promotion goes through the PING_NAT check again (JS leaves this as a TODO).
    pub(super) fn restore_firewalled(&mut self) {
        let firewalled = self.config.firewalled;
        self.io.set_firewalled(firewalled);
        self.node_nat.close(firewalled);
        self.publish_mode();
    }

    pub(super) fn set_adaptive(&mut self, on: bool) {
        match (on, self.io.ephemeral) {
            (true, true) => self.adaptive.enable(),
            (true, false) => {}
            (false, _) => {
                if self.adaptive.disable() && !self.io.ephemeral {
                    self.demote();
                }
            }
        }
        self.publish_mode();
    }

    pub(super) fn publish_mode(&self) {
        let next = DhtMode {
            ephemeral: self.io.ephemeral,
            adaptive: self.adaptive.is_on(),
            firewalled: self.io.is_firewalled(),
            address: self.node_address.clone(),
        };
        self.mode_tx.send_if_modified(|current| {
            let changed = *current != next;
            if changed {
                *current = next;
            }
            changed
        });
    }

    /// PING_NAT answers the requester's host at the port named in the value, so the reply
    /// proves reachability of that port rather than of the port the request came from.
    pub(super) fn reply_ping_nat(&mut self, req: &IncomingRequest, port: u16) {
        let to = Ipv4Peer {
            host: req.from.host.clone(),
            port,
        };
        let ctx = req.reply_ctx.clone();
        self.io
            .send_reply_deferred(&to, ctx, req.tid, req.target, 0, None);
    }
}

#[cfg(test)]
#[path = "node_adaptive_tests.rs"]
mod tests;
