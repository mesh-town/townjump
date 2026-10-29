//! Client-side `HyperDhtHandle::connect*`: entry points, node search and the handshake through one node.
//! Neighbours: `connect_flow` / `connect_race` / `connect_error` (search), `connect_finish` completes the connection.

use libudx::UdxRuntime;

use crate::crypto::hash;
use crate::holepuncher::Holepuncher;
use crate::hyperdht_messages::PEER_HANDSHAKE;
use crate::messages::Ipv4Peer;
use crate::noise_wrap::NoiseWrap;
use crate::router::Router;
use crate::rpc::Session;
use crate::rpc::UserRequestParams;

use super::config::ConnectOpts;
use super::connect_finish::AttemptError;
use super::connect_flow::Handshaken;
use super::connect_payload::client_payload;
use super::connect_route::Via;
use super::connect_trace::trace;
use super::connection::PeerConnection;
use super::handle::HyperDhtHandle;
use super::keypair::KeyPair;
use super::relay::RelayAsk;
use super::stream::next_stream_id;
use super::types::HyperDhtError;

impl HyperDhtHandle {
    // ── CONNECT (client-side holepunch orchestration) ─────────────────────

    /// Connect to a remote peer using the DHT and relay fallback.
    pub async fn connect(
        &self,
        key_pair: &KeyPair,
        remote_public_key: [u8; 32],
        runtime: &UdxRuntime,
    ) -> Result<PeerConnection, HyperDhtError> {
        self.connect_with_nodes(key_pair, remote_public_key, &[], runtime)
            .await
    }

    /// Connect to a remote peer, optionally using known relay addresses first.
    ///
    /// Connection strategy (matches Node.js `findAndConnect` in
    /// `hyperdht/lib/connect.js`, see `connect_flow.rs`):
    /// 1. The provided `relay_addresses` are tried one after another, as one
    ///    of the parallel attempts (optimistic pre-connect).
    /// 2. FIND_PEER on hash(remote_pk); each responder that returned a stored
    ///    peer record is a known-good PEER_HANDSHAKE forwarder. Its
    ///    `reply.from`, then each `peer.relay_addresses` entry of the records,
    ///    are tried while the pre-connect is already running.
    /// 3. Fallback to FIND_NODE walk on hash(remote_pk) for nodes the DHT
    ///    routing table knows about but didn't store a record.
    ///
    /// At most two handshakes (three with `relay_addresses`) run at once; the
    /// first that succeeds wins and the rest are dropped, a handshake to a
    /// silent holder gives up after ~4 s (dht-rpc defaults), as in JS.
    ///
    /// When every attempt failed the error follows `findAndConnect`: no holder of
    /// the record was tried → [`HyperDhtError::PeerNotFound`]; holders were tried
    /// → [`HyperDhtError::NoRelayNodes`] (`PEER_CONNECTION_FAILED`); a failure JS
    /// does not swallow (`FirewallRejected`, holepunch) is returned as is and
    /// ends the search (`connect_error.rs`). With a `RelayThrough::List` whose every relay
    /// aborted the pairing on our side the error is [`HyperDhtError::RelayAborted`].
    pub async fn connect_with_nodes(
        &self,
        key_pair: &KeyPair,
        remote_public_key: [u8; 32],
        relay_addresses: &[Ipv4Peer],
        runtime: &UdxRuntime,
    ) -> Result<PeerConnection, HyperDhtError> {
        self.connect_with_options(
            key_pair,
            remote_public_key,
            relay_addresses,
            runtime,
            ConnectOpts::default(),
        )
        .await
    }

    /// Like [`Self::connect_with_nodes`] but accepts a [`ConnectOpts`] options bag.
    ///
    /// Use this when you need to disable the same-NAT LAN-shortcut
    /// (e.g. to force the real-network code path under test).
    pub async fn connect_with_options(
        &self,
        key_pair: &KeyPair,
        remote_public_key: [u8; 32],
        relay_addresses: &[Ipv4Peer],
        runtime: &UdxRuntime,
        opts: ConnectOpts,
    ) -> Result<PeerConnection, HyperDhtError> {
        self.ensure_running()?;
        self.connect_candidates(key_pair, remote_public_key, relay_addresses, runtime, opts)
            .await
    }

    /// One attempt: the route of an earlier connection (only in the first attempt), then the
    /// explicit addresses or the cached ones of the target, then the holder search.
    pub(super) async fn connect_attempt(
        &self,
        key_pair: &KeyPair,
        remote_public_key: [u8; 32],
        relay_addresses: &[Ipv4Peer],
        runtime: &UdxRuntime,
        opts: &ConnectOpts,
        first: bool,
    ) -> Result<PeerConnection, AttemptError> {
        // JS `connect.js:171-190`: the route of an earlier connection first, FIND_PEER on failure.
        if first
            && opts.reusable_socket
            && let Some(conn) = self
                .retry_route(key_pair, &remote_public_key, runtime, opts)
                .await
        {
            return Ok(conn);
        }
        // JS `connect.js:353-377`: explicit addresses first, else the cached ones of the target.
        let cached;
        let pre = if relay_addresses.is_empty() {
            cached = self
                .relay_cache
                .lock()
                .map(|c| c.get(&remote_public_key))
                .unwrap_or_default();
            let hit = if cached.is_empty() { "miss" } else { "hit" };
            tracing::debug!("connect: pre-relays n={} (cache {hit})", cached.len());
            &cached[..]
        } else {
            relay_addresses
        };
        self.find_and_connect(key_pair, remote_public_key, pre, runtime, opts)
            .await
    }

    /// Connect directly to a peer at a known address, bypassing DHT routing.
    ///
    /// Sends a PEER_HANDSHAKE directly to `target_addr` for `remote_public_key`.
    /// Useful when the target's address is already known (e.g. from prior
    /// configuration or out-of-band exchange), avoiding the FIND_NODE phase
    /// that requires the target to be well-propagated in the DHT.
    pub async fn connect_to(
        &self,
        key_pair: &KeyPair,
        remote_public_key: [u8; 32],
        target_addr: std::net::SocketAddr,
        runtime: &UdxRuntime,
    ) -> Result<PeerConnection, HyperDhtError> {
        self.ensure_running()?;
        let relay = Ipv4Peer {
            host: target_addr.ip().to_string(),
            port: target_addr.port(),
        };
        let session = self.dht.session();
        self.in_session(&session)
            .connect_through_node(
                &session,
                key_pair,
                &remote_public_key,
                &relay,
                runtime,
                &ConnectOpts::default(),
            )
            .await
    }

    async fn connect_through_node(
        &self,
        session: &Session,
        key_pair: &KeyPair,
        remote_public_key: &[u8; 32],
        relay: &Ipv4Peer,
        runtime: &UdxRuntime,
        opts: &ConnectOpts,
    ) -> Result<PeerConnection, HyperDhtError> {
        let handshaken = self
            .handshake_through_node(key_pair, remote_public_key, relay, None, opts)
            .await?;
        // JS `connect.js:547`: the session's other requests stop; it stays usable.
        session.destroy();
        self.finish_connection(handshaken, key_pair, remote_public_key, runtime, opts)
            .await
    }

    /// JS `connectThroughNode`: PEER_HANDSHAKE to `relay` and the Noise reply. `opts` carries
    /// what the client announces: `reusable_socket` in its UDX info and, with `local_connection`, its
    /// local addresses. Cancel-safe: the
    /// racing attempts of `find_and_connect` are dropped when another one wins, and the session
    /// of the connect cancels their network requests. A dropped future drops `via`, which unregisters
    /// its stream.
    pub(super) async fn handshake_through_node(
        &self,
        key_pair: &KeyPair,
        remote_public_key: &[u8; 32],
        relay: &Ipv4Peer,
        via: Option<Via>,
        opts: &ConnectOpts,
    ) -> Result<Handshaken, HyperDhtError> {
        let target = hash(remote_public_key);

        // Phase 1: Noise IK handshake via PEER_HANDSHAKE relay
        let mut nw = NoiseWrap::new_initiator(key_pair.to_noise_keypair(), *remote_public_key);

        let local_stream_id = via
            .as_ref()
            .map_or_else(next_stream_id, |v| v.stream.local_id());

        // Advertise our reflexive address (if known) + local IPv4
        // interface addresses. Mirrors Node `lib/connect.js:420-437`. The
        // receiver sequentially probes this list and applies its own
        // LAN-special-case filtering — see `hyperdht/lib/server.js:426`.
        // Local addresses only with `local_connection` (JS `c.lan`).
        let t0 = tokio::time::Instant::now();
        trace("local_port await", Some(relay), t0);
        let client_addresses4 = match self.dht.local_port().await {
            Ok(p) => {
                trace("local_port done; reflexive await", Some(relay), t0);
                let reflexive = self.remote_address().await.ok().flatten();
                trace("reflexive done", Some(relay), t0);
                hello_addresses4(
                    reflexive,
                    Holepuncher::local_addresses(p),
                    opts.local_connection,
                )
            }
            Err(_) => {
                trace("local_port failed", Some(relay), t0);
                vec![]
            }
        };

        let relay_ask = opts.relay_ask.as_ref().map(RelayAsk::for_handshake);
        let hello_opts = ConnectOpts {
            relay_ask: relay_ask.clone(),
            ..opts.clone()
        };
        let local_payload = client_payload(client_addresses4, local_stream_id, &hello_opts);

        let noise_bytes = nw.send(&local_payload)?;
        let handshake_value = Router::encode_client_handshake(noise_bytes, None, None)?;

        let params = UserRequestParams {
            token: None,
            command: PEER_HANDSHAKE,
            target: Some(target),
            value: Some(handshake_value),
            // dht-rpc defaults (1 s x 3 retries ~ 4 s), as JS `dht.request`
            timeout_ms: None,
            retries: None,
        };
        trace("PEER_HANDSHAKE send", Some(relay), t0);
        // `via`: straight to the server over the socket of an earlier connection (JS `reusableSocket`).
        let resp = match via {
            Some(ref via) => {
                self.dht
                    .request_via(params, &relay.host, relay.port, via.socket.clone())
                    .await?
            }
            None => self.dht.request(params, &relay.host, relay.port).await?,
        };

        trace("PEER_HANDSHAKE reply", Some(relay), t0);
        if resp.error != 0 {
            return Err(HyperDhtError::HandshakeFailed(format!(
                "error code {}",
                resp.error
            )));
        }

        let reply_value = resp
            .value
            .ok_or_else(|| HyperDhtError::HandshakeFailed("empty reply".into()))?;

        let hs_result = {
            let router = self
                .router
                .lock()
                .map_err(|_| HyperDhtError::ChannelClosed)?;
            router.validate_handshake_reply(&reply_value, relay, &resp.from)?
        };

        let remote_payload = nw.recv(&hs_result.noise)?;
        let nw_result = nw.finalize()?;

        if remote_payload.error != 0 {
            return Err(HyperDhtError::FirewallRejected);
        }

        Ok(Handshaken {
            relay: relay.clone(),
            nw_result,
            remote_payload,
            hs_result,
            local_stream_id,
            relay_ask,
            via,
        })
    }
}

/// Addresses the client announces in its hello: the reflexive one (if known) first, then the local
/// interface addresses, which only go out with `local_connection` (JS `c.lan`).
fn hello_addresses4(
    reflexive: Option<Ipv4Peer>,
    local: Vec<Ipv4Peer>,
    local_connection: bool,
) -> Vec<Ipv4Peer> {
    let mut out: Vec<Ipv4Peer> = reflexive.into_iter().collect();
    if local_connection {
        out.extend(local);
    }
    out
}

#[cfg(test)]
#[path = "connect_tests.rs"]
mod tests;
