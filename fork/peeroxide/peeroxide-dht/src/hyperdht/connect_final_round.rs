// JS: hyperdht/lib/connect.js (roundPunch)
//! Final holepunch round (`punching=true`) of the client: one request, then the local punch loop
//! raced against `Connected` / `Aborted` and the global deadline.

use std::net::SocketAddr;

use tokio::sync::mpsc::UnboundedReceiver;

use libudx::{UdxRuntime, UdxSocket};

use crate::holepuncher::{HolepunchEvent, Holepuncher};
use crate::hyperdht_messages::NoisePayload;
use crate::messages::Ipv4Peer;
use crate::noise_wrap::NoiseWrapResult;
use crate::router::Router;
use crate::secure_payload::SecurePayload;
use crate::socket_pool::SocketPool;

use super::connect_fast_open::{
    Raced, build_punch, final_reply_error, holepunch_request, next_connected, or_connected,
    punched_result,
};
use super::connection::ConnectResult;
use super::handle::HyperDhtHandle;
use super::types::HyperDhtError;

/// Everything the final round needs from the probe rounds.
pub(super) struct FinalRound<'a> {
    pub nw_result: &'a NoiseWrapResult,
    pub remote_payload: &'a NoisePayload,
    pub relay: &'a Ipv4Peer,
    pub target: &'a [u8; 32],
    pub runtime: &'a UdxRuntime,
    pub pool: &'a SocketPool,
    pub sp: &'a SecurePayload,
    pub local_stream_id: u32,
    pub hp_id: u64,
    /// Primary socket: carries the DHT requests only; the stream uses the
    /// socket from `Connected`.
    pub via: Option<UdxSocket>,
    pub deadline: tokio::time::Instant,
    pub verified_addr: Ipv4Peer,
    pub remote_token: Option<[u8; 32]>,
    pub round: u64,
    pub addresses: Vec<Ipv4Peer>,
}

impl HyperDhtHandle {
    pub(super) async fn run_final_round(
        &self,
        fr: FinalRound<'_>,
        puncher: &mut Holepuncher,
        event_rx: &mut UnboundedReceiver<HolepunchEvent>,
    ) -> Result<ConnectResult, HyperDhtError> {
        if let Some((addr, socket)) = self.send_final_request(&fr, puncher, event_rx).await? {
            return Ok(punched_result(
                fr.nw_result,
                fr.remote_payload,
                fr.local_stream_id,
                addr,
                Some(socket),
            ));
        }
        race_punch(fr, puncher, event_rx).await
    }

    /// Sends the `punching=true` request and folds the reply into `puncher`.
    /// `Some`: `Connected` arrived first (fast mode), no local punch needed; carries the address and
    /// the socket that received the probe.
    async fn send_final_request(
        &self,
        fr: &FinalRound<'_>,
        puncher: &mut Holepuncher,
        event_rx: &mut UnboundedReceiver<HolepunchEvent>,
    ) -> Result<Option<(SocketAddr, UdxSocket)>, HyperDhtError> {
        let final_outbound_token: [u8; 32] = rand::random();
        let punch_payload = build_punch(
            puncher.nat.firewall,
            fr.round,
            fr.addresses.clone(),
            &fr.verified_addr,
            final_outbound_token,
            fr.remote_token,
        );

        let encrypted_punch = fr.sp.encrypt(&punch_payload)?;
        let hp_punch_value = Router::encode_client_holepunch(fr.hp_id, encrypted_punch, None)?;

        // `Connected` may arrive before the reply (fast mode): it ends the round at once.
        let request = holepunch_request(
            &self.dht,
            fr.target,
            hp_punch_value,
            fr.relay,
            fr.via.clone(),
        );
        let punch_resp = match or_connected(request, event_rx).await? {
            Raced::Connected(addr, socket) => {
                tracing::debug!(round = fr.round, "connected before final request");
                return Ok(Some((addr, socket)));
            }
            Raced::Done(resp) => resp?,
        };
        tracing::info!(
            target: "peeroxide::_events::holepunch::final_punch_sent",
            round = fr.round,
            verified_addr = %format!("{}:{}", fr.verified_addr.host, fr.verified_addr.port),
            "final punch sent, waiting for Connected event"
        );

        if let Some(reply_value) = &punch_resp.value {
            let hp_result = {
                let router = self
                    .router
                    .lock()
                    .map_err(|_| HyperDhtError::ChannelClosed)?;
                router.validate_holepunch_reply(
                    reply_value,
                    fr.relay,
                    &punch_resp.from,
                    fr.relay,
                )?
            };

            if let Ok(reply_hp) = fr.sp.decrypt(&hp_result.payload) {
                if let Err(e) = final_reply_error(reply_hp.error) {
                    puncher.destroy();
                    return Err(e);
                }
                let verified_host = if reply_hp.remote_token == Some(final_outbound_token) {
                    Some(hp_result.peer_address.host.as_str())
                } else {
                    None
                };
                if let Some(addrs) = &reply_hp.addresses {
                    puncher.update_remote(
                        reply_hp.punching,
                        reply_hp.firewall,
                        addrs,
                        verified_host,
                    );
                }
            }
        }
        Ok(None)
    }
}

/// Spawns the local punch loop and races it against Connected / Aborted
/// events and the global deadline. The pinned `punch_fut` mutably borrows
/// `puncher` for the duration of the inner scope; on timeout we exit the scope
/// so the borrow is released before we call `puncher.destroy()`.
async fn race_punch(
    fr: FinalRound<'_>,
    puncher: &mut Holepuncher,
    event_rx: &mut UnboundedReceiver<HolepunchEvent>,
) -> Result<ConnectResult, HyperDhtError> {
    // As in Node connect.js `onsocket`: the stream runs on the socket that
    // received the peer's probe — the peer's NAT opened its hole towards that
    // socket's mapping. The clone in `Connected` keeps it alive after the
    // puncher drops.
    let timed_out = {
        let punch_fut = puncher.punch(fr.pool, fr.runtime);
        tokio::pin!(punch_fut);
        let mut punch_done = false;
        let deadline_sleep = tokio::time::sleep_until(fr.deadline);
        tokio::pin!(deadline_sleep);

        loop {
            tokio::select! {
                res = next_connected(event_rx) => {
                    return res.map(|(addr, socket)| {
                        punched_result(
                            fr.nw_result,
                            fr.remote_payload,
                            fr.local_stream_id,
                            addr,
                            Some(socket),
                        )
                    });
                }
                _ = punch_fut.as_mut(), if !punch_done => {
                    punch_done = true;
                    tracing::debug!("local punch loop completed, awaiting Connected event");
                }
                _ = deadline_sleep.as_mut() => {
                    break true;
                }
            }
        }
    };

    if timed_out {
        tracing::debug!("punch deadline elapsed");
        puncher.destroy();
        return Err(HyperDhtError::HolepunchFailed);
    }

    unreachable!("holepunch select loop exited without return or timeout");
}

#[cfg(test)]
#[path = "connect_final_round_tests.rs"]
mod tests;
