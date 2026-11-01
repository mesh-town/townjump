// JS: none (townjump extension): connect by key races the DHT path against a LAN multicast probe.
// See also: dht.rs (`connect` calls `connect` here), node.rs (`Lan` responder, `table_size`), townjump-lan (probe protocol).
use crate::error::map_error;
use crate::node::Node;
use peeroxide_dht::hyperdht::{ConnectOpts as ForkConnectOpts, PeerConnection};
use std::future::{Future, pending};
use std::net::SocketAddr;
use std::time::Duration;
use tokio::sync::Notify;
use tokio::time::{Instant, sleep};
use townjump_dht::DhtError;

/// When the LAN probe of a `connect` starts. The rest of the race does not depend on the choice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) enum ProbeStart {
    /// (a) At once.
    Now,
    /// (b) At once if the DHT table is known empty; while bootstrap runs — after `SILENCE` with the table still empty;
    /// a table that fills means a live DHT: no probe until the DHT path fails.
    Quiet,
    /// (c) Only after the DHT path fails.
    AfterFailure,
}

/// The variant in force (question 381 of the owner; default (b)).
pub(crate) const PROBE_START: ProbeStart = ProbeStart::Quiet;
/// How long an empty table during bootstrap counts as silence.
pub(crate) const SILENCE: Duration = Duration::from_secs(1);
const POLL: Duration = Duration::from_millis(100);

/// What the trigger looks at: the end of bootstrap and the routing table size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Table {
    pub bootstrapped: bool,
    pub size: usize,
}

/// Resolves when the probe should start; `failed` is notified when the DHT path fails.
pub(crate) async fn wait_start<F, Fut>(mode: ProbeStart, mut table: F, failed: &Notify)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Table>,
{
    match mode {
        ProbeStart::Now => {}
        ProbeStart::AfterFailure => failed.notified().await,
        ProbeStart::Quiet => {
            tokio::select! {
                () = quiet(&mut table) => {}
                () = failed.notified() => {}
            }
        }
    }
}

async fn quiet<F, Fut>(table: &mut F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Table>,
{
    let begun = Instant::now();
    loop {
        let seen = table().await;
        if seen.size > 0 {
            return pending().await;
        }
        if seen.bootstrapped || begun.elapsed() >= SILENCE {
            return;
        }
        sleep(POLL).await;
    }
}

/// Connect by key: the DHT path (A) against the LAN probe (B); the first connection wins, the loser is dropped.
/// An error is always A's; B's errors are `debug` lines. If A fails first, B is awaited to the end of its window.
pub(crate) async fn connect(
    node: &Node,
    remote: [u8; 32],
    opts: ForkConnectOpts,
) -> Result<PeerConnection, DhtError> {
    if !node.lan.setup.enabled {
        return Box::pin(dht_path(node, remote, opts)).await;
    }
    node.check_alive()?;
    let failed = Notify::new();
    let dht = async {
        let result = Box::pin(dht_path(node, remote, opts)).await;
        if result.is_err() {
            failed.notify_one();
        }
        result
    };
    // Boxed: the futures are large and the callers run on small test stacks.
    race(Box::pin(dht), Box::pin(lan_path(node, remote, &failed))).await
}

async fn dht_path(
    node: &Node,
    remote: [u8; 32],
    opts: ForkConnectOpts,
) -> Result<PeerConnection, DhtError> {
    node.ready().await?;
    let connected =
        node.handle
            .connect_with_options(&node.identity, remote, &[], &node.runtime, opts);
    connected.await.map_err(map_error)
}

async fn lan_path(node: &Node, remote: [u8; 32], failed: &Notify) -> Option<PeerConnection> {
    let state = || async {
        Table {
            bootstrapped: node.bootstrap_done().await,
            size: node.table_size().await,
        }
    };
    wait_start(node.lan.setup.start, state, failed).await;
    let target = peeroxide_dht::crypto::hash(&remote);
    let addr = townjump_lan::find(&node.lan.setup.config, target, &node.lan.setup.timing).await?;
    let dialed =
        node.handle
            .connect_to(&node.identity, remote, SocketAddr::V4(addr), &node.runtime);
    dialed
        .await
        .inspect_err(|e| tracing::debug!("lan: dialing {addr} failed: {e}"))
        .ok()
}

async fn race<A, B>(dht: A, lan: B) -> Result<PeerConnection, DhtError>
where
    A: Future<Output = Result<PeerConnection, DhtError>>,
    B: Future<Output = Option<PeerConnection>>,
{
    tokio::pin!(dht, lan);
    let mut dht_error = None;
    let mut lan_open = true;
    loop {
        tokio::select! {
            result = &mut dht, if dht_error.is_none() => match result {
                Ok(conn) => return Ok(conn),
                Err(e) => dht_error = Some(e),
            },
            found = &mut lan, if lan_open => match found {
                Some(conn) => return Ok(conn),
                None => lan_open = false,
            },
        }
        if !lan_open && let Some(e) = dht_error.take() {
            return Err(e);
        }
    }
}
