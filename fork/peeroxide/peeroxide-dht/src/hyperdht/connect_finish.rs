//! Direct-or-holepunch decision and `HyperDhtHandle::finish_connection`.
//! Neighbours: `connect_flow` calls `finish_connection`; `connect_holepunch` and `connect_relayed` do the two branches.

use libudx::UdxRuntime;

use crate::crypto::hash;
use crate::hyperdht_messages::{FIREWALL_OPEN, NoisePayload};
use crate::messages::Ipv4Peer;

use super::config::ConnectOpts;
use super::connect_flow::Handshaken;
use super::connect_lan::lan_for;
use super::connect_relayed::{PunchCtx, Relayed};
use super::connect_trace::trace;
use super::connection::{ConnectResult, PeerConnection};
use super::handle::HyperDhtHandle;
use super::keypair::KeyPair;
use super::lan_pick::{local_v4_nets, pick_lan_addr};
use super::relay::client_role;
use super::stream::establish_stream_with_socket;
use super::types::HyperDhtError;

/// Matches Node.js `isBogon` from the `bogon` package — returns true for
/// loopback, link-local, private RFC-1918, and other reserved ranges.
pub(crate) fn is_addr_private(host: &str) -> bool {
    let Ok(ip) = host.parse::<std::net::Ipv4Addr>() else {
        return true;
    };
    ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_unspecified()
        || ip.is_documentation()
        || ip.octets()[0] == 100 && ip.octets()[1] >= 64 && ip.octets()[1] <= 127 // CGN
}

/// Determines whether a direct connection should be attempted instead of holepunching.
///
/// Returns `true` (direct connect) when ANY of:
/// - The handshake was NOT relayed (client reached server directly)
/// - Server reports FIREWALL_OPEN
/// - Server has no holepunch relays (can't holepunch even if we wanted to)
///
/// Peers behind one public address take the hole-punch branch like any others; their private
/// addresses are probed inside it.
pub fn should_direct_connect(relayed: bool, firewall: u64, remote_holepunchable: bool) -> bool {
    !relayed || firewall == FIREWALL_OPEN || !remote_holepunchable
}

/// Client-side "same host as the server" check: our sampled public host equals the server's.
pub(crate) fn client_same_host(
    local_connection: bool,
    ours: Option<&Ipv4Peer>,
    server_host: &str,
) -> bool {
    local_connection && ours.is_some_and(|o| o.host == server_host)
}

impl HyperDhtHandle {
    /// JS `connect.js:346,404,508`: the relay addresses of the payload, else everything the
    /// candidates handed out, are kept for the next connect to the same target. Relay-through
    /// and empty lists write nothing.
    pub(super) fn cache_relays(
        &self,
        remote_public_key: &[u8; 32],
        payload: &NoisePayload,
        tried: &[Ipv4Peer],
    ) {
        if payload.relay_through.is_some() {
            return;
        }
        let (relays, src) = match payload.relay_addresses.as_deref() {
            Some(r) if !r.is_empty() => (r.to_vec(), "payload"),
            _ => (tried.to_vec(), "tried"),
        };
        if relays.is_empty() {
            return;
        }
        tracing::debug!("connect: relay cache set n={} src={src}", relays.len());
        if let Ok(mut cache) = self.relay_cache.lock() {
            cache.set(*remote_public_key, relays);
        }
    }

    /// JS `holepunch` after `findAndConnect`: relay-through, direct dial or holepunch rounds
    /// over the handshake that won. Its failures are final (no other holder is tried).
    pub(super) async fn finish_connection(
        &self,
        mut handshaken: Handshaken,
        key_pair: &KeyPair,
        remote_public_key: &[u8; 32],
        runtime: &UdxRuntime,
        opts: &ConnectOpts,
    ) -> Result<PeerConnection, HyperDhtError> {
        // A retry by route: the server answered over the socket of an earlier connection.
        if handshaken.remote_payload.relay_through.is_none()
            && let Some(via) = handshaken.via.take()
        {
            return self.finish_via(handshaken, via).await;
        }
        // Relay-through ends the route retry: free the stream registered on the route socket now.
        drop(handshaken.via.take());
        let Handshaken {
            relay,
            nw_result,
            remote_payload,
            hs_result,
            local_stream_id,
            relay_ask,
            ..
        } = handshaken;
        let relay = &relay;
        let target = hash(remote_public_key);

        // The relay the server asked for wins over ours; either gives a role in the pairing.
        let role = client_role(remote_payload.relay_through.as_ref(), relay_ask.as_ref());

        let reusable = opts.reusable_socket
            && remote_payload
                .udx
                .as_ref()
                .is_some_and(|u| u.reusable_socket);

        // Skip holepunching when the remote peer is directly reachable.
        // Node.js (connect.js) checks:
        //   payload.firewall === FIREWALL.OPEN  -- server says it's open
        //   (relayed && !remoteHolepunchable)   -- relayed but server has no HP relays
        //   same NAT (own remote addr matches server's reflexive)  -- LAN shortcut
        // In any of those cases, connect directly using the server address from the handshake.
        let remote_holepunchable = remote_payload
            .holepunch
            .as_ref()
            .is_some_and(|hp| !hp.relays.is_empty());

        // NAT-derived same-host check. `hs_result.client_address.host` is the
        // FE-holder's source IP (not our own public address), so comparing it
        // to `server_address.host` is always false on relayed handshakes and
        // cannot be the same-host signal. Use our own NAT-sampled public
        // address instead. Disabled when the caller opts out via
        // `local_connection=false`.
        let same_host = if !opts.local_connection {
            false
        } else {
            let t0 = tokio::time::Instant::now();
            trace("remote_address await", Some(relay), t0);
            let ours = self.dht.remote_address().await.ok().flatten();
            let same = client_same_host(true, ours.as_ref(), &hs_result.server_address.host);
            trace("remote_address done", Some(relay), t0);
            same
        };

        tracing::debug!(
            relayed = hs_result.relayed,
            firewall = remote_payload.firewall,
            remote_holepunchable,
            same_host,
            server_address = %format!("{}:{}", hs_result.server_address.host, hs_result.server_address.port),
            "handshake complete, deciding connection path"
        );

        if should_direct_connect(
            hs_result.relayed,
            remote_payload.firewall,
            remote_holepunchable,
        ) {
            // Pick the dial target. When same-host is true (both peers
            // share a NAT), prefer a private/loopback address from the
            // remote's advertised list, since the public IP often can't be
            // reached from inside the same NAT (no NAT hairpin). Otherwise
            // prefer a non-private public address, falling back to the
            // FE-holder's-tagged server address. Mirrors Node
            // `lib/connect.js::holepunch` LAN-shortcut.
            // Prefers an advertised address on one of our own subnets, so
            // the reply source matches the saved route.
            let connect_addr = if same_host {
                pick_lan_addr(&remote_payload.addresses4, &local_v4_nets())
                    .or_else(|| {
                        remote_payload
                            .addresses4
                            .iter()
                            .find(|a| is_addr_private(&a.host))
                            .cloned()
                    })
                    .or_else(|| {
                        remote_payload
                            .addresses4
                            .iter()
                            .find(|a| !is_addr_private(&a.host))
                            .cloned()
                    })
                    .unwrap_or_else(|| hs_result.server_address.clone())
            } else {
                remote_payload
                    .addresses4
                    .iter()
                    .find(|a| !is_addr_private(&a.host))
                    .cloned()
                    .unwrap_or_else(|| hs_result.server_address.clone())
            };
            tracing::debug!(
                same_host,
                connect_addr = %format!("{}:{}", connect_addr.host, connect_addr.port),
                server_address = %format!("{}:{}", hs_result.server_address.host, hs_result.server_address.port),
                advertised = ?remote_payload.addresses4.iter().map(|a| format!("{}:{}", a.host, a.port)).collect::<Vec<_>>(),
                "direct-connect dial target chosen"
            );

            let direct = ConnectResult {
                remote_public_key: nw_result.remote_public_key,
                server_address: connect_addr.clone(),
                client_address: hs_result.client_address,
                is_relayed: false,
                noise: nw_result,
                local_stream_id,
                remote_udx: remote_payload.udx.clone(),
                udx_socket: None,
            };
            if let Some(role) = role {
                let keep_alive = opts.relay_keep_alive;
                self.spawn_background_relay(key_pair, runtime, role, local_stream_id, keep_alive);
            }
            let shared = self.server_socket().await?;
            let conn = establish_stream_with_socket(&direct, runtime, shared.clone()).await?;
            // The route on the node's own socket: nothing to hold, so no lease.
            if let Some(socket) = shared.filter(|_| reusable) {
                self.reuse
                    .add_route(*remote_public_key, &socket, connect_addr);
            }
            return Ok(conn);
        }

        // Phase 2: holepunch rounds (with the private candidates probed next to them on a shared
        // public address), raced against the relay when there is a role.
        let lan = lan_for(same_host, &remote_payload.addresses4, opts);
        let ctx = PunchCtx {
            nw_result,
            remote_payload,
            relay: relay.clone(),
            target,
            server_address: hs_result.server_address.clone(),
            local_stream_id,
            lan,
        };
        let Some(role) = role else {
            let hp_result = self
                .punch_or_lan(&ctx, runtime, opts.holepunch.as_ref())
                .await?;
            return self
                .punched_connection(&hp_result, None, runtime, reusable, remote_public_key)
                .await;
        };
        // Boxed: the pairing connects to the relay, which is a connect of its own.
        match Box::pin(self.connect_relayed(key_pair, ctx, role, runtime, opts)).await? {
            Relayed::Over(conn) => Ok(conn),
            Relayed::Punched { result, stream } => {
                self.punched_connection(&result, Some(stream), runtime, reusable, remote_public_key)
                    .await
            }
        }
    }
}
