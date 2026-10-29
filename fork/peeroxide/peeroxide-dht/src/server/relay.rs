// JS: hyperdht/lib/server.js
//! Relay role of the server: the relay we announce and who initiates the pair
//! (JS `server.js` 370-390 announcement, 412-421 decision).

use std::time::{Duration, Instant};

use libudx::UdxRuntime;
use tokio::sync::mpsc::{self, UnboundedSender};

use super::actor::{Internal, State};
use super::conn::{self, Pending};
use super::opts::ServerOpts;
use super::plan::Plan;
use super::punch::PunchState;
use crate::hyperdht::{
    AbortOnDrop, DirectSource, PeerConnection, RelayAddress, RelayPaired, RelayPath, RelayThrough,
    Upgrade, pair_via_relay,
};
use crate::hyperdht_messages::RelayThroughInfo;
use crate::messages::Ipv4Peer;

/// How long a slot waits for the relay pair: the DHT lookup plus the 15 s pairing.
pub(super) const RELAY_HOLD: Duration = Duration::from_secs(30);

/// Relay, token and role of one relayed connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RelayPlan {
    pub(super) relay_pk: [u8; 32],
    pub(super) token: [u8; 32],
    pub(super) is_initiator: bool,
}

/// The relay this handshake announces, with a fresh token; `None` when the options name none.
/// A client that already named a relay from our own `List` keeps it: no own relay then.
pub(super) fn own_relay(
    opts: &ServerOpts,
    remote: Option<&RelayThroughInfo>,
) -> Option<RelayThroughInfo> {
    let through = opts.relay_through.as_ref()?;
    if let (RelayThrough::List(keys), Some(r)) = (through, remote) {
        if keys.contains(&r.public_key) {
            return None;
        }
    }
    let public_key = through.select()?;
    Some(RelayThroughInfo {
        version: 1,
        public_key,
        token: rand::random::<[u8; 32]>(),
    })
}

/// The relay to use: our own (we initiate), else the client's (we do not), else none.
/// Exception to the JS rule: `own_relay` yields nothing when the client already named a relay
/// from our own `List`, so that choice stands.
pub(super) fn choose(
    own: Option<&RelayThroughInfo>,
    remote: Option<&RelayThroughInfo>,
) -> Option<RelayPlan> {
    if let Some(o) = own {
        return Some(plan_of(o, true));
    }
    Some(plan_of(remote?, false))
}

fn plan_of(info: &RelayThroughInfo, is_initiator: bool) -> RelayPlan {
    RelayPlan {
        relay_pk: info.public_key,
        token: info.token,
        is_initiator,
    }
}

/// A client that is open or asked directly gets no relay (JS 412-417).
pub(super) fn relay_allowed(plan: Plan) -> bool {
    plan != Plan::DialClient
}

/// The pairing task; dropping it closes the channel to the relay.
type RelayTask = AbortOnDrop<()>;

/// A relay pairing in progress for one pending connection.
pub(super) struct RelayAttempt {
    pub(super) plan: RelayPlan,
    /// Until when the slot waits for the relay.
    pub(super) deadline: Instant,
    task: Option<RelayTask>,
}

/// The attempt for a handshake that waits for the client: none for an open or direct client.
pub(super) fn attempt(
    plan: Plan,
    own: Option<&RelayThroughInfo>,
    remote: Option<&RelayThroughInfo>,
) -> Option<RelayAttempt> {
    if !relay_allowed(plan) {
        return None;
    }
    Some(RelayAttempt {
        plan: choose(own, remote)?,
        deadline: Instant::now() + RELAY_HOLD,
        task: None,
    })
}

/// Spawns the pairing; its outcome comes back as `Internal::RelayPaired`.
pub(super) fn start(
    st: &State,
    attempt: &mut RelayAttempt,
    id: u64,
    local_id: u32,
    internal: &UnboundedSender<Internal>,
) {
    let dht = st.dht.clone();
    let key = st.relay_key.clone();
    let rt = UdxRuntime::shared(st.rt.handle());
    let (plan, keep_alive) = (attempt.plan.clone(), st.opts.relay_keep_alive);
    let addrs = relay_addresses(&st.opts.relay_through_addresses, &plan.relay_pk);
    let tx = internal.clone();
    let task = tokio::spawn(async move {
        let paired = pair_via_relay(
            &dht,
            &key,
            &rt,
            plan.relay_pk,
            &addrs,
            plan.token,
            plan.is_initiator,
            local_id,
            keep_alive,
        )
        .await;
        let _ = tx.send(Internal::RelayPaired { id, paired });
    });
    attempt.task = Some(AbortOnDrop(task));
}

/// Configured addresses of the relay `key`, in order (same rule as `addresses_for`).
fn relay_addresses(list: &[RelayAddress], key: &[u8; 32]) -> Vec<Ipv4Peer> {
    let same = list.iter().filter(|a| &a.public_key == key);
    same.map(|a| a.addr.clone()).collect()
}

/// A pending connection whose pair is made: its puzzle stays alive while the relay carries the data.
pub(super) struct Relayed {
    pub(super) punch: PunchState,
    pub(super) deadline: Instant,
}

/// Serves the paired connection over the relay; a failure of any step drops everything.
pub(super) async fn finish_relayed(
    p: Pending,
    paired: RelayPaired,
    id: u64,
    internal: UnboundedSender<Internal>,
    tx: mpsc::Sender<PeerConnection>,
) {
    if serve_relayed(p, paired, id, &internal, tx).await.is_none() {
        let _ = internal.send(Internal::RelayDone { id });
    }
}

async fn serve_relayed(
    p: Pending,
    paired: RelayPaired,
    id: u64,
    internal: &UnboundedSender<Internal>,
    tx: mpsc::Sender<PeerConnection>,
) -> Option<()> {
    let RelayPaired {
        socket,
        relay_addr,
        remote_id,
        session,
    } = paired;
    let connected = p.stream.connect(&socket, remote_id, relay_addr).await;
    connected
        .inspect_err(|e| tracing::debug!(err = %e, "server: relay connect failed"))
        .ok()?;
    let control = p.stream.control();
    let (addr, _) = control.remote()?;
    if addr != relay_addr {
        tracing::debug!(%addr, "server: client arrived first, relay dropped");
        drop(session);
        conn::finish(p, addr, tx).await;
        return Some(());
    }
    let closed = control.closed();
    let direct = DirectSource::PathHook {
        socket: p.socket.clone(),
        remote_id: p.remote_udx_id,
    };
    let link = Upgrade::spawn(control, session, direct);
    let mut path = link.path();
    let secret = conn::session(p.stream, &p.noise).await?;
    let key = p.noise.remote_public_key;
    let mut conn = PeerConnection::with_remote_addr(secret, key, relay_addr, socket, None);
    conn.set_relay(link);
    let done = internal.clone();
    tokio::spawn(async move {
        tokio::select! {
            _ = closed => {}
            _ = path.wait_for(|now| *now == RelayPath::Direct) => {}
        }
        let _ = done.send(Internal::RelayDone { id });
    });
    if tx.try_send(conn).is_err() {
        tracing::debug!("server: incoming channel is full or closed, relayed connection dropped");
    }
    Some(())
}

#[cfg(test)]
#[path = "relay_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "relay_choose_tests.rs"]
mod relay_choose_tests;
