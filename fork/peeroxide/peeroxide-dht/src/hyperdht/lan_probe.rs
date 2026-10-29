//! Reachability check of one private candidate: a one-byte hole-punch probe answered by the
//! server's passive puncher, and a DHT ping answered by its node socket; the first answer counts.
//! Neighbours: `connect_lan` drives it.

use std::net::SocketAddr;

use libudx::{UdxRuntime, UdxSocket};
use tokio::sync::mpsc;

use crate::holepuncher::{HolepunchEvent, Holepuncher};
use crate::messages::Ipv4Peer;
use crate::socket_pool::SocketPool;

use super::connect_lan::{LAN_PROBE_RESEND, LanProbe, first_some};
use super::handle::HyperDhtHandle;
use super::types::HyperDhtError;

/// An initiating puncher used only to send probes and hear their reflection.
pub(crate) struct LanProber {
    puncher: Holepuncher,
    events: mpsc::UnboundedReceiver<HolepunchEvent>,
}

impl LanProber {
    pub(crate) async fn new(
        runtime: &UdxRuntime,
        remote_firewall: u64,
    ) -> Result<Self, HyperDhtError> {
        let pool = SocketPool::new("0.0.0.0".into());
        let (tx, events) = mpsc::unbounded_channel();
        let puncher = Holepuncher::new(&pool, runtime, true, true, remote_firewall, tx)
            .await
            .map_err(|_| HyperDhtError::HolepunchFailed)?;
        Ok(Self { puncher, events })
    }

    /// The socket the probes leave from.
    pub(crate) fn socket(&self) -> Option<UdxSocket> {
        self.puncher.primary_socket().map(|sr| sr.socket.clone())
    }

    fn send(&self, peer: &Ipv4Peer) -> Option<()> {
        let primary = self.puncher.primary_socket()?;
        primary.send_holepunch_to(&peer.host, peer.port, false).ok()
    }

    /// Probes every `LAN_PROBE_RESEND` until a reflection arrives; gives the address it came from.
    pub(crate) async fn reach(&mut self, peer: &Ipv4Peer) -> Option<SocketAddr> {
        let mut tick = tokio::time::interval(LAN_PROBE_RESEND);
        loop {
            tokio::select! {
                _ = tick.tick() => self.send(peer)?,
                event = self.events.recv() => return match event {
                    Some(HolepunchEvent::Connected { addr, .. }) => Some(addr),
                    _ => None,
                },
            }
        }
    }
}

/// The prober together with the node's DHT handle for the ping path.
pub(crate) struct LanProbes<'a> {
    prober: LanProber,
    handle: &'a HyperDhtHandle,
}

impl<'a> LanProbes<'a> {
    pub(crate) async fn new(
        handle: &'a HyperDhtHandle,
        runtime: &UdxRuntime,
        remote_firewall: u64,
    ) -> Result<Self, HyperDhtError> {
        let prober = LanProber::new(runtime, remote_firewall).await?;
        Ok(Self { prober, handle })
    }
}

type Hit = (SocketAddr, Option<UdxSocket>);

async fn ping_candidate(handle: &HyperDhtHandle, peer: &Ipv4Peer) -> Option<Hit> {
    let addr: SocketAddr = format!("{}:{}", peer.host, peer.port).parse().ok()?;
    handle.dht.ping(&peer.host, peer.port).await.ok()?;
    Some((addr, None))
}

impl LanProbe for LanProbes<'_> {
    type Hit = Hit;

    async fn reach(&mut self, peer: &Ipv4Peer) -> Option<Hit> {
        let prober = &mut self.prober;
        let probe = async {
            let addr = prober.reach(peer).await?;
            Some((addr, prober.socket()))
        };
        first_some(probe, ping_candidate(self.handle, peer)).await
    }
}

#[cfg(test)]
#[path = "lan_probe_tests.rs"]
mod tests;
