// JS: hyperdht/lib/holepuncher.js (autoSample, analyze)
//! Background NAT sampling of a [`Holepuncher`]: pongs flow over a channel and
//! are merged into `nat` before each probe and inside `analyze`.

use super::Holepuncher;
use crate::hyperdht_messages::FIREWALL_UNKNOWN;
use crate::nat::NAT_MIN_SAMPLES;
use crate::nat_sampler::{spawn_dht_reply_forwarder, spawn_sampler};
use crate::rpc::DhtHandle;
use tokio::sync::mpsc::error::TryRecvError;

impl Holepuncher {
    /// Starts the reply forwarder and, if the NAT is still unclassified, the
    /// sampler; returns at once. No-op without a primary socket or when its
    /// `dht_reply_rx` was already taken.
    pub fn start_background_sampling(&mut self, dht: &DhtHandle) {
        let Some(socket_ref) = self.sockets.first_mut() else {
            return;
        };
        let Some(rx) = socket_ref.dht_reply_rx.take() else {
            return;
        };
        socket_ref.serve_dht_requests(dht);
        let socket = socket_ref.socket.clone();
        spawn_dht_reply_forwarder(dht.clone(), rx);
        if self.nat.firewall == FIREWALL_UNKNOWN {
            self.samples = Some(spawn_sampler(dht, socket, NAT_MIN_SAMPLES as usize));
        }
    }

    /// Merges the pongs that have already arrived into `nat`; never blocks.
    pub(crate) fn absorb_samples(&mut self) {
        loop {
            let Some(rx) = self.samples.as_mut() else {
                return;
            };
            match rx.try_recv() {
                Ok((to, from)) => self.nat.add(&to, &from),
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => return self.finish_sampling(),
            }
        }
    }

    /// Merges pongs until `nat` settles or the sampler is done.
    pub(super) async fn wait_samples(&mut self) {
        while !self.nat.sampling_settled() {
            let Some(rx) = self.samples.as_mut() else {
                return;
            };
            match rx.recv().await {
                Some((to, from)) => self.nat.add(&to, &from),
                None => self.finish_sampling(),
            }
        }
    }

    fn finish_sampling(&mut self) {
        self.samples = None;
        self.nat.finish_sampling();
    }
}

#[cfg(test)]
#[path = "sampling_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "dht_request_tests.rs"]
mod dht_request_tests;
