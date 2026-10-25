//! Private-address candidates raced against the hole-punch rounds when both peers share one
//! public address. Neighbours: `connect_finish` builds the candidates, `lan_probe` checks one.

use std::future::Future;
use std::net::Ipv4Addr;
#[cfg(test)]
use std::sync::Arc;
use std::time::Duration;

use libudx::UdxRuntime;

use crate::messages::Ipv4Peer;

use super::config::ConnectOpts;
use super::connect_fast_open::punched_result;
use super::connect_finish::is_addr_private;
use super::connect_relayed::PunchCtx;
use super::connection::ConnectResult;
use super::handle::HyperDhtHandle;
use super::lan_pick::{local_v4_nets, pick_lan_addr};
use super::lan_probe::LanProbes;
use super::relay::HolepunchFilter;
use super::types::HyperDhtError;

/// How long one candidate has to answer.
pub(crate) const LAN_PROBE_DEADLINE: Duration = Duration::from_secs(2);
/// Gap between repeated probes to the same candidate.
pub(super) const LAN_PROBE_RESEND: Duration = Duration::from_millis(500);

/// Test lever: replaces the candidates computed from the server's advertisement.
#[cfg(test)]
pub(crate) type RewriteFn = dyn Fn(&[Ipv4Peer]) -> Vec<Ipv4Peer> + Send + Sync;

#[cfg(test)]
#[derive(Clone)]
pub(crate) struct LanRewrite(pub(crate) Arc<RewriteFn>);

#[cfg(test)]
impl std::fmt::Debug for LanRewrite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LanRewrite(..)")
    }
}

/// The advertised private addresses: the one on one of our own subnets first, then the rest in
/// advertised order. Public addresses are never candidates.
pub(super) fn lan_candidates(
    advertised: &[Ipv4Peer],
    local_nets: &[(Ipv4Addr, u8)],
) -> Vec<Ipv4Peer> {
    let mut out: Vec<Ipv4Peer> = pick_lan_addr(advertised, local_nets).into_iter().collect();
    for peer in advertised.iter().filter(|a| is_addr_private(&a.host)) {
        if !out.contains(peer) {
            out.push(peer.clone());
        }
    }
    out
}

#[cfg(test)]
fn rewritten(opts: &ConnectOpts, advertised: &[Ipv4Peer]) -> Option<Vec<Ipv4Peer>> {
    opts.lan_rewrite.as_ref().map(|r| (r.0)(advertised))
}

#[cfg(not(test))]
fn rewritten(_opts: &ConnectOpts, _advertised: &[Ipv4Peer]) -> Option<Vec<Ipv4Peer>> {
    None
}

/// Candidates for this connect: none unless the peers share a public address.
pub(super) fn lan_for(
    same_host: bool,
    advertised: &[Ipv4Peer],
    opts: &ConnectOpts,
) -> Vec<Ipv4Peer> {
    if !same_host {
        return Vec::new();
    }
    rewritten(opts, advertised).unwrap_or_else(|| lan_candidates(advertised, &local_v4_nets()))
}

/// Checks one candidate; there is no deadline of its own.
pub(crate) trait LanProbe {
    type Hit: Send;

    /// `None`: no answer will come (send failed, channel closed).
    fn reach(&mut self, peer: &Ipv4Peer) -> impl Future<Output = Option<Self::Hit>> + Send;
}

/// Candidates in order, each under [`LAN_PROBE_DEADLINE`]; the first answer wins.
pub(super) async fn first_reachable<P: LanProbe>(
    candidates: &[Ipv4Peer],
    probe: &mut P,
) -> Option<P::Hit> {
    for peer in candidates {
        if let Ok(Some(hit)) = tokio::time::timeout(LAN_PROBE_DEADLINE, probe.reach(peer)).await {
            return Some(hit);
        }
    }
    None
}

/// The first `Some` of two futures; a `None` waits for the other. The loser is dropped.
pub(super) async fn first_some<T>(
    a: impl Future<Output = Option<T>>,
    b: impl Future<Output = Option<T>>,
) -> Option<T> {
    tokio::pin!(a, b);
    tokio::select! {
        biased;
        r = &mut a => match r {
            Some(v) => Some(v),
            None => b.await,
        },
        r = &mut b => match r {
            Some(v) => Some(v),
            None => a.await,
        },
    }
}

/// LAN against hole punching, LAN first on ties. The loser is dropped. A failed side leaves the
/// other to decide; both failed gives the punch error.
pub(super) async fn lan_or_punch<T>(
    lan: impl Future<Output = Option<T>>,
    punch: impl Future<Output = Result<T, HyperDhtError>>,
) -> Result<T, HyperDhtError> {
    tokio::pin!(lan, punch);
    tokio::select! {
        biased;
        l = &mut lan => match l {
            Some(v) => Ok(v),
            None => punch.await,
        },
        p = &mut punch => match p {
            Ok(v) => Ok(v),
            Err(e) => lan.await.ok_or(e),
        },
    }
}

impl HyperDhtHandle {
    /// The hole-punch rounds, with the private candidates probed beside them when there are any.
    pub(super) async fn punch_or_lan(
        &self,
        ctx: &PunchCtx,
        runtime: &UdxRuntime,
        filter: Option<&HolepunchFilter>,
    ) -> Result<ConnectResult, HyperDhtError> {
        let punch = self.run_holepunch_rounds(
            &ctx.nw_result,
            &ctx.remote_payload,
            &ctx.relay,
            &ctx.target,
            &ctx.server_address,
            runtime,
            ctx.local_stream_id,
            filter,
        );
        if ctx.lan.is_empty() {
            return punch.await;
        }
        lan_or_punch(reach_lan(self, ctx, runtime), punch).await
    }
}

async fn reach_lan(
    handle: &HyperDhtHandle,
    ctx: &PunchCtx,
    runtime: &UdxRuntime,
) -> Option<ConnectResult> {
    tracing::debug!(candidates = ?ctx.lan, "lan dial: probing candidates");
    let made = LanProbes::new(handle, runtime, ctx.remote_payload.firewall).await;
    let mut probes = made
        .inspect_err(|e| tracing::debug!(error = %e, "lan dial: prober unavailable"))
        .ok()?;
    let Some((addr, socket)) = first_reachable(&ctx.lan, &mut probes).await else {
        tracing::debug!("lan dial: no candidate answered");
        return None;
    };
    let via = if socket.is_some() { "puncher" } else { "node" };
    tracing::debug!(%addr, via, "lan dial: candidate answered");
    Some(punched_result(
        &ctx.nw_result,
        &ctx.remote_payload,
        ctx.local_stream_id,
        addr,
        socket,
    ))
}

#[cfg(test)]
#[path = "connect_lan_tests.rs"]
mod tests;
