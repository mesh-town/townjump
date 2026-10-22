//! `HyperDhtHandle::run_holepunch_rounds`: the client-side holepunch rounds (one long function).
//! Neighbours: `connect_finish` calls it; `connect_relayed` is the fallback; fast mode — `connect_fast_open`.

use std::net::SocketAddr;

use tokio::sync::mpsc;

use libudx::{UdxRuntime, UdxSocket};

use crate::holepuncher::Holepuncher;
use crate::hyperdht_messages::{
    ERROR_ABORTED, ERROR_NONE, ERROR_TRY_LATER, FIREWALL_UNKNOWN, NoisePayload,
};
use crate::messages::Ipv4Peer;
use crate::noise_wrap::NoiseWrapResult;
use crate::router::Router;
use crate::secure_payload::SecurePayload;
use crate::socket_pool::SocketPool;

use super::connect_fast_open::{
    Raced, add_probe_sample, build_probe, connected_now, grace_for_unknown, or_connected,
    punched_result, send_probe,
};
use super::connect_final_round::FinalRound;
use super::connection::ConnectResult;
use super::handle::HyperDhtHandle;
use super::relay::{HolepunchFilter, gate_punch};
use super::types::HyperDhtError;

impl HyperDhtHandle {
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn run_holepunch_rounds(
        &self,
        nw_result: &NoiseWrapResult,
        remote_payload: &NoisePayload,
        relay: &Ipv4Peer,
        target: &[u8; 32],
        server_address: &Ipv4Peer,
        runtime: &UdxRuntime,
        local_stream_id: u32,
        filter: Option<&HolepunchFilter>,
    ) -> Result<ConnectResult, HyperDhtError> {
        // Node-shaped multi-round client holepunch loop (JS `lib/connect.js`):
        //   1. Up to `HOLEPUNCH_MAX_PROBE_ROUNDS` probe rounds (`punching=false`); each reply feeds
        //      `update_remote(...)`, then `analyze(false)` says whether the local NAT is settled.
        //   2. A reply is "verified" only when its `remote_token` echoes the token we sent that round.
        //   3. With a verified address and a settled NAT: one final `punching=true` round
        //      (`run_final_round`), bounded by `HOLEPUNCH_TOTAL_DEADLINE`.
        //   4. `reply.error`: NONE continues, TRY_LATER retries next round, ABORTED returns
        //      `HolepunchAborted`, anything else `HolepunchFailed`.

        /// Maximum wall-clock time spent in the full holepunch flow (probes +
        /// final punch + waiting for Connected).
        const HOLEPUNCH_TOTAL_DEADLINE: std::time::Duration = std::time::Duration::from_secs(30);
        /// Maximum number of `punching=false` probe rounds before we either
        /// transition to the final punch round or give up.
        const HOLEPUNCH_MAX_PROBE_ROUNDS: u32 = 3;

        let sp = SecurePayload::new(nw_result.holepunch_secret);
        let pool = SocketPool::new("0.0.0.0".into());
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();

        let hp_id = remote_payload.holepunch.as_ref().map_or(0, |hp| hp.id);

        let mut puncher = Holepuncher::new(
            &pool,
            runtime,
            true,
            true,
            remote_payload.firewall,
            event_tx,
        )
        .await
        .map_err(|_| HyperDhtError::HolepunchFailed)?;

        // JS `holepuncher.js:19-20`: sampling runs in the background; the round-0 probe needs no samples.
        puncher.start_background_sampling(&self.dht);

        let via: Option<UdxSocket> = puncher.primary_socket().map(|sr| sr.socket.clone());

        let deadline = tokio::time::Instant::now() + HOLEPUNCH_TOTAL_DEADLINE;

        // State carried between probe rounds.
        let mut cached_reply_token: Option<[u8; 32]> = None;
        let mut verified_remote_addr: Option<Ipv4Peer> = None;
        let mut last_round: u64 = 0;
        let mut gated = false;

        for round in 0u64..=u64::from(HOLEPUNCH_MAX_PROBE_ROUNDS) {
            last_round = round;

            // Fresh outbound token per round; the reply must echo this back
            // in `remote_token` for us to consider the host verified.
            let our_outbound_token: [u8; 32] = rand::random();

            // Samples that arrived meanwhile change the addresses we advertise.
            puncher.absorb_samples();
            let probe_payload = build_probe(
                puncher.nat.firewall,
                round,
                puncher.punch_addresses(),
                server_address,
                our_outbound_token,
                cached_reply_token,
            );

            let encrypted_probe = sp.encrypt(&probe_payload)?;
            let hp_value = Router::encode_client_holepunch(hp_id, encrypted_probe, None)?;

            tracing::debug!(round, hp_id, "probe sent");

            // The server's packet (`Connected`) ends the punch without waiting for the relay's reply;
            // dropping the request future cancels it, a late reply is ignored (JS `isDone`).
            let probe = send_probe(
                &self.dht,
                &puncher,
                server_address,
                target,
                hp_value,
                relay,
                via.clone(),
            );
            let hp_resp = match or_connected(probe, &mut event_rx).await? {
                Raced::Connected(addr) => {
                    // Deviation from JS: a veto also stops an early `Connected`.
                    gate_punch(filter, &mut puncher)?;
                    let by = "packet_before_reply";
                    return Ok(fast_mode_result(
                        nw_result,
                        remote_payload,
                        local_stream_id,
                        addr,
                        via,
                        round,
                        by,
                    ));
                }
                Raced::Done(r) => r?,
            };

            if hp_resp.error != 0 {
                puncher.destroy();
                return Err(HyperDhtError::HolepunchFailed);
            }

            let Some(reply_value) = &hp_resp.value else {
                puncher.destroy();
                return Err(HyperDhtError::HolepunchFailed);
            };

            let hp_result = {
                let router = self
                    .router
                    .lock()
                    .map_err(|_| HyperDhtError::ChannelClosed)?;
                router.validate_holepunch_reply(reply_value, relay, &hp_resp.from, relay)?
            };

            let reply_hp = sp.decrypt(&hp_result.payload)?;

            match reply_hp.error {
                ERROR_NONE => {}
                ERROR_TRY_LATER => {
                    tracing::debug!(round, "reply ERROR_TRY_LATER, continuing");
                    cached_reply_token = reply_hp.token;
                    continue;
                }
                ERROR_ABORTED => {
                    puncher.destroy();
                    return Err(HyperDhtError::HolepunchAborted);
                }
                other => {
                    tracing::warn!(round, error = other, "holepunch reply server error");
                    puncher.destroy();
                    return Err(HyperDhtError::HolepunchFailed);
                }
            }

            // Mirror Node `connect.js:623`: feed NAT with the reflexive `to`, not the server address.
            add_probe_sample(&mut puncher.nat, &hp_resp);

            // Token-echo verification: only consider the host verified if
            // the reply echoes back our outbound token in `remote_token`.
            let token_matches = reply_hp.remote_token == Some(our_outbound_token);
            let verified_host = if token_matches {
                Some(hp_result.peer_address.host.as_str())
            } else {
                None
            };

            tracing::debug!(
                round,
                punching = reply_hp.punching,
                firewall = reply_hp.firewall,
                addresses = reply_hp.addresses.as_ref().map_or(0, |a| a.len()),
                verified = token_matches,
                "probe reply"
            );

            if let Some(addrs) = &reply_hp.addresses {
                puncher.update_remote(reply_hp.punching, reply_hp.firewall, addrs, verified_host);
            }

            if let Some(addr) = connected_now(&mut event_rx) {
                gate_punch(filter, &mut puncher)?;
                let by = "after_reply";
                return Ok(fast_mode_result(
                    nw_result,
                    remote_payload,
                    local_stream_id,
                    addr,
                    via,
                    round,
                    by,
                ));
            }

            // JS `connect.js:321`: the filter sees the peer once, after its first answer.
            if !std::mem::replace(&mut gated, true) {
                gate_punch(filter, &mut puncher)?;
            }

            if token_matches {
                verified_remote_addr = Some(hp_result.peer_address.clone());
            }
            cached_reply_token = reply_hp.token;

            // Same race for the 1 s UNKNOWN grace: `Connected` does not wait for a round.
            let grace = or_connected(grace_for_unknown(reply_hp.firewall), &mut event_rx);
            if let Raced::Connected(addr) = grace.await? {
                let by = "during_grace";
                return Ok(fast_mode_result(
                    nw_result,
                    remote_payload,
                    local_stream_id,
                    addr,
                    via,
                    round,
                    by,
                ));
            }

            // `analyze` merges the background samples (bounded by the 2 s ping timeout).
            if verified_remote_addr.is_some() && reply_hp.firewall != FIREWALL_UNKNOWN {
                // `Connected` (fast mode) ends the round even while the samples are still pending.
                if let Raced::Connected(addr) =
                    or_connected(puncher.analyze(false), &mut event_rx).await?
                {
                    let by = "during_analyze";
                    return Ok(fast_mode_result(
                        nw_result,
                        remote_payload,
                        local_stream_id,
                        addr,
                        via,
                        round,
                        by,
                    ));
                }
                if puncher.nat.is_settled() {
                    tracing::info!(
                        target: "peeroxide::_events::holepunch::nat_settled",
                        round,
                        "NAT settled + verified remote, transitioning to final punch round"
                    );
                    break;
                }
            }
        }

        // The last round may end in `continue` (TRY_LATER) with `Connected` already queued.
        if let Some(addr) = connected_now(&mut event_rx) {
            let by = "after_rounds";
            return Ok(fast_mode_result(
                nw_result,
                remote_payload,
                local_stream_id,
                addr,
                via,
                last_round,
                by,
            ));
        }

        let Some(verified_addr) = verified_remote_addr else {
            tracing::warn!(
                target: "peeroxide::_events::holepunch::failed_no_verified_addr",
                "no verified remote address after probe rounds"
            );
            puncher.destroy();
            return Err(HyperDhtError::HolepunchFailed);
        };

        // JS `connect.js:645`; no-op when the samples are already merged.
        puncher.analyze(false).await;
        puncher.absorb_samples();
        tracing::debug!(
            added = puncher.nat.sampled,
            firewall = puncher.nat.firewall,
            "initiator auto_sample"
        );

        let fr = FinalRound {
            nw_result,
            remote_payload,
            relay,
            target,
            runtime,
            pool: &pool,
            sp: &sp,
            local_stream_id,
            hp_id,
            via,
            deadline,
            verified_addr,
            remote_token: cached_reply_token,
            round: last_round.saturating_add(1),
            addresses: puncher.punch_addresses(),
        };
        self.run_final_round(fr, &mut puncher, &mut event_rx).await
    }
}

/// Early success (fast mode): logs the round and the path (`by`), then builds the result.
fn fast_mode_result(
    nw_result: &NoiseWrapResult,
    remote_payload: &NoisePayload,
    local_stream_id: u32,
    addr: SocketAddr,
    via: Option<UdxSocket>,
    round: u64,
    by: &'static str,
) -> ConnectResult {
    tracing::debug!(round, by, "connected during probe rounds (fast mode)");
    punched_result(nw_result, remote_payload, local_stream_id, addr, via)
}

#[cfg(test)]
#[path = "connect_holepunch_tests.rs"]
mod tests;
