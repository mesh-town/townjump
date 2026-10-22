//! Client pairing through a blind relay: `pair_via_relay`, `RelayPaired`, `RelaySession`.
//! The caller creates the data stream (it owns the stream id) and connects it via the relay.
// JS: hyperdht/lib/connect.js:793-838

use std::net::SocketAddr;
use std::time::Duration;

use libudx::{UdxRuntime, UdxSocket};
use tokio::task::JoinHandle;

use super::super::handle::HyperDhtHandle;
use super::super::keypair::KeyPair;
use super::super::reuse::Lease;
use super::super::types::HyperDhtError;
use crate::blind_relay::BlindRelayClient;
use crate::protomux::Mux;

const PAIR_TIMEOUT: Duration = Duration::from_secs(15);
const MUX_STOP_WAIT: Duration = Duration::from_secs(2);

/// A completed pairing: the socket to the relay, its address and the relay-side stream id.
pub(crate) struct RelayPaired {
    pub socket: UdxSocket,
    pub relay_addr: SocketAddr,
    pub remote_id: u32,
    pub session: RelaySession,
}

/// Owns the control connection to the relay; closing or dropping it ends the pairing.
pub(crate) struct RelaySession {
    live: Option<Live>,
    #[cfg(test)]
    signal: Option<tokio::sync::oneshot::Sender<()>>,
}

/// The parts of a real session (a test fake has none).
struct Live {
    client: BlindRelayClient,
    token: [u8; 32],
    mux: Mux,
    task: JoinHandle<()>,
    _socket: UdxSocket,
    _lease: Option<Lease>,
}

impl RelaySession {
    /// Unpair, close the channel and the mux, and wait up to 2 s for the mux task.
    pub(crate) async fn close(mut self) {
        let Some(live) = self.live.take() else {
            return;
        };
        let mut task = live.stop();
        if tokio::time::timeout(MUX_STOP_WAIT, &mut task)
            .await
            .is_err()
        {
            task.abort();
        }
    }

    fn wrap(live: Live) -> Self {
        Self {
            live: Some(live),
            #[cfg(test)]
            signal: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn probe() -> (Self, tokio::sync::oneshot::Receiver<()>) {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let session = Self {
            live: None,
            signal: Some(tx),
        };
        (session, rx)
    }
}

impl Live {
    /// Best-effort shutdown; returns the mux task so the caller may wait for it.
    fn stop(mut self) -> JoinHandle<()> {
        if let Err(e) = self.client.unpair(&self.token) {
            tracing::debug!(err = %e, "relay: unpair failed");
        }
        self.client.close();
        drop(self.mux);
        self.task
    }
}

impl Drop for RelaySession {
    fn drop(&mut self) {
        #[cfg(test)]
        if let Some(tx) = self.signal.take() {
            let _ = tx.send(());
        }
        if let Some(live) = self.live.take() {
            live.stop().abort();
        }
    }
}

/// Connect to the relay and pair `token`; any failure or the 15 s limit is `RelayAborted`.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn pair_via_relay(
    dht: &HyperDhtHandle,
    key_pair: &KeyPair,
    runtime: &UdxRuntime,
    relay_pk: [u8; 32],
    token: [u8; 32],
    is_initiator: bool,
    local_stream_id: u32,
    keep_alive: Duration,
) -> Result<RelayPaired, HyperDhtError> {
    let chain = pair_chain(
        dht,
        key_pair,
        runtime,
        relay_pk,
        token,
        is_initiator,
        local_stream_id,
        keep_alive,
    );
    match tokio::time::timeout(PAIR_TIMEOUT, chain).await {
        Ok(Ok(paired)) => Ok(paired),
        Ok(Err(e)) => {
            tracing::debug!(err = %e, "relay: pairing failed");
            Err(HyperDhtError::RelayAborted)
        }
        Err(_) => {
            tracing::debug!("relay: pairing timed out");
            Err(HyperDhtError::RelayAborted)
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn pair_chain(
    dht: &HyperDhtHandle,
    key_pair: &KeyPair,
    runtime: &UdxRuntime,
    relay_pk: [u8; 32],
    token: [u8; 32],
    is_initiator: bool,
    local_stream_id: u32,
    keep_alive: Duration,
) -> Result<RelayPaired, HyperDhtError> {
    let mut conn = dht
        .connect_with_nodes(key_pair, relay_pk, &[], runtime)
        .await?;
    let relay_addr = conn.remote_addr.ok_or_else(|| {
        HyperDhtError::StreamEstablishment("relay connection has no remote_addr".into())
    })?;
    if !keep_alive.is_zero() {
        conn.stream.set_keep_alive(keep_alive);
    }
    let (socket, lease) = (conn.socket.clone(), conn.lease.take());
    let (mux, run) = Mux::new(conn.stream);
    let task = tokio::spawn(run);
    let mut client = BlindRelayClient::open(&mux, Some(key_pair.public_key.to_vec())).await?;
    // From here the session owns the shutdown, so an early error still unpairs and closes.
    let live = |client| Live {
        client,
        token,
        mux,
        task,
        _socket: socket.clone(),
        _lease: lease,
    };
    if let Err(e) = client.wait_opened().await {
        drop(RelaySession::wrap(live(client)));
        return Err(e.into());
    }
    let reply = client
        .pair(is_initiator, &token, u64::from(local_stream_id))
        .await;
    let session = RelaySession::wrap(live(client));
    let remote_id = u32::try_from(reply?.remote_id).map_err(|_| {
        HyperDhtError::StreamEstablishment("relay remote_id out of u32 range".into())
    })?;
    Ok(RelayPaired {
        socket,
        relay_addr,
        remote_id,
        session,
    })
}

#[cfg(test)]
#[path = "pair_tests.rs"]
mod tests;
