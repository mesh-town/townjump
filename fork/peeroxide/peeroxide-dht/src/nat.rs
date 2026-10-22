#![allow(dead_code)]

use crate::hyperdht_messages::{
    FIREWALL_CONSISTENT, FIREWALL_OPEN, FIREWALL_RANDOM, FIREWALL_UNKNOWN,
};
use crate::messages::Ipv4Peer;
use crate::rpc::DhtHandle;
use libudx::UdxSocket;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Notify;

pub(crate) const NAT_MIN_SAMPLES: u32 = 4;

#[derive(Debug, Clone)]
struct Sample {
    host: String,
    port: u16,
    hits: u32,
}

#[derive(Debug)]
pub struct Nat {
    samples_host: Vec<Sample>,
    samples_full: Vec<Sample>,
    visited: std::collections::HashMap<String, u8>,
    frozen: bool,
    firewalled: bool,

    pub sampled: u32,
    pub firewall: u64,
    pub addresses: Option<Vec<Ipv4Peer>>,

    analyzing_notify: Arc<Notify>,
    analyzing_settled: Arc<AtomicBool>,
}

impl Nat {
    pub fn new(firewalled: bool) -> Self {
        Self {
            samples_host: Vec::new(),
            samples_full: Vec::new(),
            visited: std::collections::HashMap::new(),
            frozen: false,
            firewalled,
            sampled: 0,
            firewall: if firewalled {
                FIREWALL_UNKNOWN
            } else {
                FIREWALL_OPEN
            },
            addresses: None,
            analyzing_notify: Arc::new(Notify::new()),
            analyzing_settled: Arc::new(AtomicBool::new(!firewalled)),
        }
    }

    pub fn destroy(&mut self) {
        self.frozen = true;
        // Force-settle on destroy. Mirrors Node `nat.js` `destroy()` calling
        // `_resolve()` unconditionally so any pending `analyzing` await
        // wakes immediately rather than hanging on a NAT that will never
        // accumulate more samples.
        if !self.analyzing_settled.swap(true, Ordering::AcqRel) {
            self.analyzing_notify.notify_waiters();
        }
    }

    pub fn freeze(&mut self) {
        self.frozen = true;
    }

    pub fn unfreeze(&mut self) {
        self.frozen = false;
        self.update_firewall();
        self.update_addresses();
    }

    pub fn update(&mut self) {
        if self.firewalled && self.firewall == FIREWALL_OPEN {
            self.firewall = FIREWALL_UNKNOWN;
        }
        self.update_firewall();
        self.update_addresses();
    }

    pub fn add(&mut self, addr: &Ipv4Peer, from: &Ipv4Peer) {
        let from_ref = format!("{}:{}", from.host, from.port);

        if self.visited.get(&from_ref) == Some(&2) {
            return;
        }
        self.visited.insert(from_ref, 2);

        add_sample(&mut self.samples_host, &addr.host, 0);
        add_sample(&mut self.samples_full, &addr.host, addr.port);

        self.sampled += 1;

        if (self.sampled >= 3 || !self.firewalled) && !self.frozen {
            self.update();
        }

        self.try_settle();
    }

    pub fn is_settled(&self) -> bool {
        self.firewall == FIREWALL_CONSISTENT || self.firewall == FIREWALL_OPEN
    }

    pub fn analyzing(&self) -> impl std::future::Future<Output = ()> + 'static {
        let notify = Arc::clone(&self.analyzing_notify);
        let settled = Arc::clone(&self.analyzing_settled);
        async move {
            if settled.load(Ordering::Acquire) {
                return;
            }
            let waiter = notify.notified();
            tokio::pin!(waiter);
            waiter.as_mut().enable();
            if settled.load(Ordering::Acquire) {
                return;
            }
            waiter.await;
        }
    }

    fn try_settle(&mut self) {
        let should = matches!(self.firewall, FIREWALL_OPEN | FIREWALL_CONSISTENT)
            || self.sampled >= NAT_MIN_SAMPLES;
        if should && !self.analyzing_settled.swap(true, Ordering::AcqRel) {
            self.analyzing_notify.notify_waiters();
        }
    }

    pub fn mark_visited(&mut self, host: &str, port: u16) -> bool {
        let key = format!("{host}:{port}");
        if self.visited.contains_key(&key) {
            return false;
        }
        self.visited.insert(key, 1);
        true
    }

    /// Seed the puncher's NAT classifier with reflexive samples by sending
    /// DHT pings out the supplied puncher socket. Mirrors Node's
    /// `lib/nat.js:25-79 autoSample()`. Returns the number of NEW samples
    /// added; short-circuits with `0` if the NAT is already settled.
    pub(crate) async fn auto_sample(
        &mut self,
        dht: &DhtHandle,
        socket: UdxSocket,
        max_samples: usize,
    ) -> usize {
        if self.firewall != FIREWALL_UNKNOWN {
            return 0;
        }
        let before = self.sampled;
        let mut rx = crate::nat_sampler::spawn_sampler(dht, socket, max_samples);
        while let Some((to, from)) = rx.recv().await {
            self.add(&to, &from);
            if self.firewall != FIREWALL_UNKNOWN {
                break;
            }
        }
        (self.sampled - before) as usize
    }

    /// True once `analyzing` is released (enough samples, or sampling ended).
    pub(crate) fn sampling_settled(&self) -> bool {
        self.analyzing_settled.load(Ordering::Acquire)
    }

    /// Release `analyzing` without the sample-count condition: sampling is over.
    pub(crate) fn finish_sampling(&mut self) {
        if !self.analyzing_settled.swap(true, Ordering::AcqRel) {
            self.analyzing_notify.notify_waiters();
        }
    }

    fn update_firewall(&mut self) {
        if !self.firewalled {
            self.firewall = FIREWALL_OPEN;
            return;
        }

        if self.sampled < 3 {
            return;
        }

        let max = match self.samples_full.first() {
            Some(s) => s.hits,
            None => return,
        };

        if max >= 3 {
            self.firewall = FIREWALL_CONSISTENT;
            return;
        }

        if max == 1 {
            self.firewall = FIREWALL_RANDOM;
            return;
        }

        // max === 2
        // 1 host, >= 4 total samples ie, 2 bad ones -> random
        if self.samples_host.len() == 1 && self.sampled > 3 {
            self.firewall = FIREWALL_RANDOM;
            return;
        }

        // double hit on two different ips -> assume consistent
        if self.samples_host.len() > 1
            && self.samples_full.len() > 1
            && self.samples_full[1].hits > 1
        {
            self.firewall = FIREWALL_CONSISTENT;
            return;
        }

        // (4 just means all the samples we expect) - no decision - assume random
        if self.sampled > 4 {
            self.firewall = FIREWALL_RANDOM;
        }
    }

    fn update_addresses(&mut self) {
        if self.firewall == FIREWALL_UNKNOWN {
            self.addresses = None;
            return;
        }

        if self.firewall == FIREWALL_RANDOM {
            if let Some(s) = self.samples_host.first() {
                self.addresses = Some(vec![Ipv4Peer {
                    host: s.host.clone(),
                    port: s.port,
                }]);
            }
            return;
        }

        if self.firewall == FIREWALL_CONSISTENT {
            let mut addrs = Vec::new();
            for s in &self.samples_full {
                if s.hits >= 2 || addrs.len() < 2 {
                    addrs.push(Ipv4Peer {
                        host: s.host.clone(),
                        port: s.port,
                    });
                }
            }
            self.addresses = Some(addrs);
        }
    }
}

fn add_sample(samples: &mut Vec<Sample>, host: &str, port: u16) {
    for i in 0..samples.len() {
        if samples[i].port != port || samples[i].host != host {
            continue;
        }

        samples[i].hits += 1;

        // Bubble up to maintain descending sort by hits
        let mut j = i;
        while j > 0 && samples[j - 1].hits < samples[j].hits {
            samples.swap(j - 1, j);
            j -= 1;
        }
        return;
    }

    samples.push(Sample {
        host: host.to_string(),
        port,
        hits: 1,
    });
}

#[cfg(test)]
#[path = "nat_tests.rs"]
mod tests;
