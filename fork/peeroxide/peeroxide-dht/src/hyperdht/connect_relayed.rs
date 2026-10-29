//! Client race between hole punching and a blind-relay pairing, plus the background relay
//! that runs next to a direct connection. Neighbours: `connect_finish` calls both.
// JS: hyperdht/lib/connect.js:498-536,793-838

use std::time::Duration;

use libudx::{UdxRuntime, UdxSocket, UdxStream};

use crate::hyperdht_messages::NoisePayload;
use crate::messages::Ipv4Peer;
use crate::noise_wrap::NoiseWrapResult;
use crate::secret_stream::SecretStream;

use super::config::ConnectOpts;
use super::connection::{ConnectResult, PeerConnection};
use super::handle::HyperDhtHandle;
use super::keypair::KeyPair;
use super::relay::{
    AbortOnDrop, RelayLink, RelayPaired, RelayRole, Upgrade, addresses_for, pair_via_relay,
};
use super::stream::{establish_on_stream, establish_stream_with_socket};
use super::types::HyperDhtError;

/// What the race gives back: a connection over the relay, or a punched path plus the stream
/// created for it (the same stream serves both paths).
pub(super) enum Relayed {
    Over(PeerConnection),
    Punched {
        result: ConnectResult,
        stream: UdxStream,
    },
}

/// Everything the punch task needs, owned (the task outlives the borrow of `finish_connection`).
#[derive(Clone)]
pub(super) struct PunchCtx {
    pub nw_result: NoiseWrapResult,
    pub remote_payload: NoisePayload,
    pub relay: Ipv4Peer,
    pub target: [u8; 32],
    pub server_address: Ipv4Peer,
    pub local_stream_id: u32,
    /// Private candidates probed next to the punch rounds; empty off the shared-address path.
    pub lan: Vec<Ipv4Peer>,
}

type PunchTask = AbortOnDrop<Result<ConnectResult, HyperDhtError>>;
type SocketTask = AbortOnDrop<Result<(ConnectResult, UdxSocket), HyperDhtError>>;

fn punched(
    joined: Result<Result<ConnectResult, HyperDhtError>, tokio::task::JoinError>,
) -> Result<ConnectResult, HyperDhtError> {
    joined.unwrap_or(Err(HyperDhtError::HolepunchFailed))
}

impl HyperDhtHandle {
    /// The tail of a punched path: the stream on the punch socket, held and remembered as a route.
    pub(super) async fn punched_connection(
        &self,
        hp_result: &ConnectResult,
        stream: Option<UdxStream>,
        runtime: &UdxRuntime,
        reusable: bool,
        remote_public_key: &[u8; 32],
    ) -> Result<PeerConnection, HyperDhtError> {
        let shared = match hp_result.udx_socket.clone() {
            Some(s) => Some(s),
            None => self.server_socket().await?,
        };
        let mut conn = match stream {
            Some(stream) => {
                let socket = match shared {
                    Some(s) => s,
                    None => {
                        let s = runtime.create_socket().await?;
                        s.bind("0.0.0.0:0".parse().expect("valid addr")).await?;
                        s
                    }
                };
                establish_on_stream(hp_result, stream, socket).await?
            }
            None => establish_stream_with_socket(hp_result, runtime, shared).await?,
        };
        // Only the punch socket is held; the node's fallback socket is not (no hold, no route).
        if let Some(socket) = hp_result.udx_socket.as_ref().filter(|_| reusable)
            && let Some(lease) = self.reuse.hold(socket)
        {
            conn.hold(lease);
            self.reuse
                .add_route(*remote_public_key, socket, hp_result.server_address.clone());
        }
        Ok(conn)
    }

    /// Punch and pair at once. The punch wins ties (`biased`); a failed side leaves the other
    /// to decide, both failed is `RelayAborted`. Reaching the relay first hands the running punch
    /// to the upgrade supervisor, which moves the stream to the punched path.
    pub(super) async fn connect_relayed(
        &self,
        key_pair: &KeyPair,
        ctx: PunchCtx,
        role: RelayRole,
        runtime: &UdxRuntime,
        opts: &ConnectOpts,
    ) -> Result<Relayed, HyperDhtError> {
        let stream = runtime.create_stream(ctx.local_stream_id).await?;
        let mut punch = self.spawn_punch(&ctx, runtime, opts);
        let addresses = addresses_for(&opts.relay_through_addresses, &role.public_key);
        let pair = pair_via_relay(
            self,
            key_pair,
            runtime,
            role.public_key,
            &addresses,
            role.token,
            role.is_initiator,
            ctx.local_stream_id,
            opts.relay_keep_alive,
        );
        tokio::pin!(pair);
        let (paired, punch) = tokio::select! {
            biased;
            p = &mut punch.0 => match punched(p) {
                Ok(result) => return Ok(Relayed::Punched { result, stream }),
                Err(_) => (pair.await?, None),
            },
            r = &mut pair => match r {
                Ok(paired) => (paired, Some(punch)),
                Err(_) => return finish_punch(punch, stream).await,
            },
        };
        let punch = punch.map(|p| self.with_socket(p));
        over_relay(&ctx, stream, paired, punch)
            .await
            .map(Relayed::Over)
    }

    /// The punch result together with the socket it punched on (the node's own when it gave none).
    fn with_socket(&self, mut punch: PunchTask) -> SocketTask {
        let this = self.clone();
        AbortOnDrop(tokio::spawn(async move {
            let result = punched((&mut punch.0).await)?;
            let shared = match result.udx_socket.clone() {
                Some(s) => Some(s),
                None => this.server_socket().await?,
            };
            let socket = shared.ok_or(HyperDhtError::HolepunchFailed)?;
            Ok((result, socket))
        }))
    }

    /// The hole punching in a session of its own (the end of the connect does not cancel it).
    fn spawn_punch(&self, ctx: &PunchCtx, runtime: &UdxRuntime, opts: &ConnectOpts) -> PunchTask {
        let session = self.dht.session();
        let this = self.in_session(&session);
        let runtime = UdxRuntime::shared(runtime.handle());
        let ctx = ctx.clone();
        let filter = opts.holepunch.clone();
        let delay = opts.punch_delay;
        AbortOnDrop(tokio::spawn(async move {
            let _session = session;
            if let Some(d) = delay {
                tokio::time::sleep(d).await;
            }
            this.punch_or_lan(&ctx, &runtime, filter.as_ref()).await
        }))
    }

    /// JS `connect.js:534` with a direct path: the pairing runs in the background and is
    /// closed as soon as it ends; the connection is not touched either way.
    pub(super) fn spawn_background_relay(
        &self,
        key_pair: &KeyPair,
        runtime: &UdxRuntime,
        role: RelayRole,
        local_stream_id: u32,
        keep_alive: Duration,
    ) {
        let (this, key_pair) = (self.clone(), key_pair.clone());
        let runtime = UdxRuntime::shared(runtime.handle());
        tokio::spawn(async move {
            let paired = pair_via_relay(
                &this,
                &key_pair,
                &runtime,
                role.public_key,
                &[],
                role.token,
                role.is_initiator,
                local_stream_id,
                keep_alive,
            )
            .await;
            if let Ok(paired) = paired {
                paired.session.close().await;
            }
        });
    }
}

/// The relay failed: the punch is the only way left.
async fn finish_punch(mut punch: PunchTask, stream: UdxStream) -> Result<Relayed, HyperDhtError> {
    match punched((&mut punch.0).await) {
        Ok(result) => Ok(Relayed::Punched { result, stream }),
        Err(_) => Err(HyperDhtError::RelayAborted),
    }
}

/// Connect the stream through the relay and wrap it with the Noise keys of the handshake.
async fn over_relay(
    ctx: &PunchCtx,
    stream: UdxStream,
    paired: RelayPaired,
    punch: Option<SocketTask>,
) -> Result<PeerConnection, HyperDhtError> {
    let RelayPaired {
        socket,
        relay_addr,
        remote_id,
        session,
    } = paired;
    stream.connect(&socket, remote_id, relay_addr).await?;
    let control = stream.control();
    let nw = &ctx.nw_result;
    let ss = SecretStream::from_session(
        nw.is_initiator,
        stream.into_async_stream(),
        nw.tx,
        nw.rx,
        nw.handshake_hash,
        nw.remote_public_key,
    )
    .await?;
    let mut conn =
        PeerConnection::with_remote_addr(ss, nw.remote_public_key, relay_addr, socket, None);
    let link = match punch {
        Some(punch) => Upgrade::spawn_after_punch(control, session, punch),
        None => RelayLink::relayed_only(control, session),
    };
    conn.set_relay(link);
    Ok(conn)
}
