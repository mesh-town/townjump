//! Client pairing through a blind relay: `pair_via_relay`, `RelayPaired`, `RelaySession`.
//! The caller creates the data stream (it owns the stream id) and connects it via the relay.
// JS: hyperdht/lib/connect.js:793-838

use std::future::Future;
use std::net::SocketAddr;
use std::time::Duration;

use libudx::{UdxRuntime, UdxSocket};
use tokio::sync::watch;

use super::super::handle::HyperDhtHandle;
use super::super::keypair::KeyPair;
use super::super::reuse::Lease;
use super::super::types::HyperDhtError;
use super::AbortOnDrop;
use crate::blind_relay::BlindRelayClient;
use crate::messages::Ipv4Peer;
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
    /// Turns `true` when the mux task ends by itself (the service session is over); never when we stop it.
    ended: watch::Receiver<bool>,
    #[cfg(test)]
    signal: Option<tokio::sync::oneshot::Sender<()>>,
}

/// The parts of a real session (a test fake has none).
struct Live {
    client: BlindRelayClient,
    token: [u8; 32],
    mux: Mux,
    task: AbortOnDrop<()>,
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
        // On timeout the guard aborts the task when it is dropped.
        let _ = tokio::time::timeout(MUX_STOP_WAIT, &mut task.0).await;
    }

    /// Resolves when the service session ended on its own (the relay closed it or its keepalive ran out).
    /// Our own `close()` or drop never resolves it; it stays pending then.
    pub(crate) fn ended(&self) -> impl Future<Output = ()> + Send + 'static {
        let mut ended = self.ended.clone();
        async move {
            if ended.wait_for(|over| *over).await.is_err() {
                std::future::pending::<()>().await;
            }
        }
    }

    fn wrap(live: Live, ended: watch::Receiver<bool>) -> Self {
        Self {
            live: Some(live),
            ended,
            #[cfg(test)]
            signal: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn probe() -> (Self, tokio::sync::oneshot::Receiver<()>) {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let (_, ended) = watch::channel(false);
        let session = Self {
            live: None,
            ended,
            signal: Some(tx),
        };
        (session, rx)
    }

    /// As `probe`, plus a trigger that makes `ended()` resolve.
    #[cfg(test)]
    pub(crate) fn probe_ending() -> (
        Self,
        tokio::sync::oneshot::Receiver<()>,
        watch::Sender<bool>,
    ) {
        let (mut session, closed) = Self::probe();
        let (end, ended) = watch::channel(false);
        session.ended = ended;
        (session, closed, end)
    }
}

impl Live {
    /// Best-effort shutdown; returns the mux task so the caller may wait for it.
    fn stop(mut self) -> AbortOnDrop<()> {
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
            drop(live.stop());
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
    addresses: &[Ipv4Peer],
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
        addresses,
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
    addresses: &[Ipv4Peer],
    token: [u8; 32],
    is_initiator: bool,
    local_stream_id: u32,
    keep_alive: Duration,
) -> Result<RelayPaired, HyperDhtError> {
    let mut conn = dht
        .connect_with_nodes(key_pair, relay_pk, addresses, runtime)
        .await?;
    let relay_addr = conn.remote_addr.ok_or_else(|| {
        HyperDhtError::StreamEstablishment("relay connection has no remote_addr".into())
    })?;
    if !keep_alive.is_zero() {
        conn.stream.set_keep_alive(keep_alive);
    }
    let (socket, lease) = (conn.socket.clone(), conn.lease.take());
    // A dead service stream ends the session too: its reader never wakes after a failed retransmission.
    let service_closed = conn.stream.get_ref().peer().closed();
    let (mux, run) = Mux::new(conn.stream);
    let (task, ended) = spawn_mux(run, service_closed, local_stream_id);
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
        drop(RelaySession::wrap(live(client), ended));
        return Err(e.into());
    }
    let reply = client
        .pair(is_initiator, &token, u64::from(local_stream_id))
        .await;
    let session = RelaySession::wrap(live(client), ended);
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

/// Runs the mux task under an abort-on-drop guard; tests track it in a registry by stream id.
/// The receiver turns `true` when the task ends by itself or `service_closed` resolves; an abort of the task
/// drops the sender without a signal.
fn spawn_mux(
    run: impl std::future::Future<Output = ()> + Send + 'static,
    service_closed: impl std::future::Future<Output = ()> + Send + 'static,
    local_stream_id: u32,
) -> (AbortOnDrop<()>, watch::Receiver<bool>) {
    let (over, ended) = watch::channel(false);
    let run = async move {
        tokio::select! {
            () = run => {}
            () = service_closed => {}
        }
        let _ = over.send(true);
    };
    #[cfg(test)]
    let guard = registry::enter(local_stream_id);
    #[cfg(not(test))]
    let _ = local_stream_id;
    #[cfg(test)]
    let task = tokio::spawn(async move {
        let _guard = guard;
        run.await
    });
    #[cfg(not(test))]
    let task = tokio::spawn(run);
    (AbortOnDrop(task), ended)
}

/// Test-only registry of live mux tasks, keyed by `local_stream_id`.
#[cfg(test)]
pub(crate) mod registry {
    use std::sync::Mutex;

    static LIVE: Mutex<Vec<u32>> = Mutex::new(Vec::new());

    /// Removes its id from the registry when the owning future ends or is dropped.
    pub(crate) struct Guard(u32);

    pub(crate) fn enter(id: u32) -> Guard {
        LIVE.lock().unwrap().push(id);
        Guard(id)
    }

    /// Whether a mux task registered under `id` is still alive.
    pub(crate) fn is_live(id: u32) -> bool {
        LIVE.lock().unwrap().contains(&id)
    }

    impl Drop for Guard {
        fn drop(&mut self) {
            let mut live = LIVE.lock().unwrap();
            if let Some(i) = live.iter().position(|v| *v == self.0) {
                live.swap_remove(i);
            }
        }
    }
}

#[cfg(test)]
#[path = "pair_cancel_tests.rs"]
mod pair_cancel_tests;

#[cfg(test)]
#[path = "pair_tests.rs"]
mod tests;
