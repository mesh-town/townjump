// JS: hyperdht/lib/server.js
//! The passive side of hole punching: the puzzle (`Holepuncher`) made at
//! handshake time and the PEER_HOLEPUNCH rounds (JS lines 516-657).

use std::time::Duration;

use libudx::{UdxRuntime, UdxSocket};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use super::conn::{FollowStream, follow_frames};
use super::reply::{abort_payload, round_payload};
use crate::holepuncher::{HolepunchEvent, Holepuncher};
use crate::hyperdht::{HolepunchFilter, HolepunchInput, HyperDhtError, HyperDhtHandle};
use crate::hyperdht_messages::{ERROR_NONE, HolepunchMessage, HolepunchPayload, RelayInfo};
use crate::messages::Ipv4Peer;
use crate::nat::Nat;
use crate::secure_payload::SecurePayload;
use crate::socket_pool::SocketPool;

/// How long the reflector of the puzzle lives after an early refusal of `punch()`: the longest
/// window of a full `punch()` (random probes plus keep-alive, about 35 seconds).
pub(super) const PUZZLE_LINGER: Duration = Duration::from_secs(35);

/// Hole-punching state of one handshake, kept next to its UDX stream.
pub(super) struct PunchState {
    /// The puzzle; taken out (and run in its own task) once the client punches.
    pub(super) puncher: Option<Holepuncher>,
    /// Reflexive samples of the puzzle: payload `addresses` (JS `p.nat.addresses`);
    /// the handshake adds interface addresses to them.
    pub(super) punch_addrs: Vec<Ipv4Peer>,
    /// Round payload cipher; known only after the Noise handshake is final (`arm`).
    secure: Option<SecurePayload>,
    round: u64,
    punching: bool,
    /// Dropping (or destroying) the state stops the running puzzle.
    pub(super) stop: Option<oneshot::Sender<()>>,
    /// Where to report a UDX frame on a puzzle socket.
    follow: Option<FollowStream>,
}

/// What the actor does with the answer of one round.
pub(super) enum Round {
    /// Answer with the encrypted payload; `punching` = the puzzle was started in this round.
    Reply { bytes: Vec<u8>, punching: bool },
    /// Tell the client to give up (encrypted payload); the actor drops the slot.
    Abort(Vec<u8>),
    /// No answer at all (unknown slot, bad ciphertext).
    Silent,
}

/// Makes the puzzle on a fresh socket pool and returns it with its primary
/// socket and port (like `swarm.rs` `try_setup_passive_holepunch`).
pub(super) async fn setup(
    dht: &HyperDhtHandle,
    rt: &UdxRuntime,
    remote_fw: u64,
) -> Result<(PunchState, UdxSocket, u16), HyperDhtError> {
    let pool = SocketPool::new("0.0.0.0".into());
    let (tx, _rx) = mpsc::unbounded_channel::<HolepunchEvent>();
    let mut puncher = Holepuncher::new(&pool, rt, true, false, remote_fw, tx)
        .await
        .map_err(|e| HyperDhtError::StreamEstablishment(format!("holepunch socket: {e}")))?;
    let added = puncher.auto_sample(dht.dht()).await;
    tracing::debug!(
        added,
        firewall = puncher.nat.firewall,
        "server punch auto_sample"
    );
    match socket_and_port(&puncher).await {
        Ok((socket, port)) => {
            let punch_addrs = puncher.punch_addresses();
            let state = PunchState::new(puncher, punch_addrs);
            Ok((state, socket, port))
        }
        Err(e) => {
            puncher.destroy();
            Err(e)
        }
    }
}

async fn socket_and_port(puncher: &Holepuncher) -> Result<(UdxSocket, u16), HyperDhtError> {
    let no_socket = || HyperDhtError::StreamEstablishment("puncher has no primary socket".into());
    let socket = puncher.primary_socket().map(|s| s.socket.clone());
    let socket = socket.ok_or_else(no_socket)?;
    let port = socket.local_addr().await?.port();
    Ok((socket, port))
}

/// Runs the puzzle in its own task. When `punch()` refuses early, the reflector keeps answering
/// until the returned sender is dropped or `linger` passes; then the puzzle is destroyed (an
/// abandoned puzzle would keep its socket alive through the reflector).
fn spawn_puzzle(
    mut puncher: Holepuncher,
    linger: Duration,
    follow: Option<FollowStream>,
) -> oneshot::Sender<()> {
    let (stop, stop_rx) = oneshot::channel::<()>();
    tokio::spawn(async move {
        let (pool, watcher) = puzzle_pool(follow);
        let Ok(rt) = UdxRuntime::new() else {
            return puncher.destroy();
        };
        if !puncher.punch(&pool, &rt).await {
            tracing::debug!(
                firewall = puncher.nat.firewall,
                "server: puzzle punch returned false, lingering"
            );
            tokio::select! {
                _ = stop_rx => {}
                _ = tokio::time::sleep(linger) => {}
            }
        }
        puncher.destroy();
        // Without a frame the watcher would keep the sink open, and with it the sockets.
        if let Some(w) = watcher {
            w.abort();
        }
    });
    stop
}

/// The pool of the running puzzle. With `follow`, a watcher reports the first UDX frame that
/// reaches any of its sockets.
fn puzzle_pool(follow: Option<FollowStream>) -> (SocketPool, Option<JoinHandle<()>>) {
    let pool = SocketPool::new("0.0.0.0".into());
    let Some(f) = follow else {
        return (pool, None);
    };
    let (tx, rx) = mpsc::unbounded_channel();
    (
        pool.with_udx_frames(tx),
        Some(tokio::spawn(follow_frames(f, rx))),
    )
}

impl PunchState {
    fn new(puncher: Holepuncher, punch_addrs: Vec<Ipv4Peer>) -> Self {
        Self {
            puncher: Some(puncher),
            punch_addrs,
            secure: None,
            round: 0,
            punching: false,
            stop: None,
            follow: None,
        }
    }

    pub(super) fn follow(&mut self, f: FollowStream) {
        self.follow = Some(f);
    }

    /// The puzzle socket that got the client's first UDX frame, if any.
    pub(super) fn moved_socket(&self) -> Option<UdxSocket> {
        self.follow.as_ref().and_then(FollowStream::moved_socket)
    }

    /// Sets the round cipher from the final handshake's holepunch secret.
    pub(super) fn arm(&mut self, holepunch_secret: [u8; 32]) {
        self.secure = Some(SecurePayload::new(holepunch_secret));
    }

    /// Destroys the puzzle if it is still here; `stop` goes away with `self`.
    pub(super) fn destroy(self) {
        if let Some(mut puncher) = self.puncher {
            puncher.destroy();
        }
    }

    fn open(&self, payload: &[u8], host: &str) -> Option<(HolepunchPayload, [u8; 32])> {
        let secure = self.secure.as_ref()?;
        Some((secure.decrypt(payload).ok()?, secure.token(host)))
    }

    fn seal(&self, payload: &HolepunchPayload) -> Option<Vec<u8>> {
        self.secure.as_ref()?.encrypt(payload).ok()
    }

    fn observe(&mut self, remote: &HolepunchPayload, verified: Option<&str>) {
        if remote.round < self.round {
            return;
        }
        self.round = remote.round;
        let addresses = remote.addresses.as_deref().unwrap_or(&[]);
        if let Some(puncher) = self.puncher.as_mut() {
            puncher.update_remote(remote.punching, remote.firewall, addresses, verified);
        }
    }

    /// Sends the fast-mode ping from the puzzle socket; nothing if the puzzle is gone.
    fn fast_ping(&self, remote_address: Option<&Ipv4Peer>, peer_address: &Ipv4Peer) {
        let Some(puncher) = self.puncher.as_ref() else {
            return;
        };
        let Some(to) = fast_ping_target(&puncher.nat, remote_address, peer_address) else {
            return;
        };
        let Some(sr) = puncher.primary_socket() else {
            return;
        };
        match sr.send_holepunch_to(&to.host, to.port, false) {
            Ok(()) => {
                tracing::debug!(to = %format!("{}:{}", to.host, to.port), "server fast-mode ping")
            }
            Err(e) => tracing::debug!(err = %e, "server fast-mode ping failed"),
        }
    }

    /// The `opts.holepunch` filter said no to a punching client (JS server.js 578); needs a live puzzle.
    fn vetoed(&self, remote: &HolepunchPayload, filter: Option<&HolepunchFilter>) -> bool {
        let (Some(filter), true, Some(puncher)) = (filter, remote.punching, self.puncher.as_ref())
        else {
            return false;
        };
        !filter.allows(&HolepunchInput {
            remote_firewall: remote.firewall,
            local_firewall: puncher.nat.firewall,
            remote_addresses: remote.addresses.as_deref().unwrap_or(&[]),
            local_addresses: &self.punch_addrs,
        })
    }

    /// Runs the puzzle in its own task when the client punches; `true` = started now.
    fn start_punch(&mut self, remote_punching: bool) -> bool {
        if !remote_punching {
            return false;
        }
        let Some(puncher) = self.puncher.take() else {
            return false;
        };
        self.stop = Some(spawn_puzzle(puncher, PUZZLE_LINGER, self.follow.clone()));
        self.punching = true;
        true
    }

    fn abort(&mut self, remote_round: u64) -> Round {
        self.round = self.round.max(remote_round);
        match self.seal(&abort_payload(self.round)) {
            Some(bytes) => Round::Abort(bytes),
            None => Round::Silent,
        }
    }
}

/// Fast mode (JS server.js:562-571): ping `peer_address` when our NAT is OPEN or
/// CONSISTENT and the client opened its session to one of our sampled addresses.
pub(super) fn fast_ping_target(
    nat: &Nat,
    remote_address: Option<&Ipv4Peer>,
    peer_address: &Ipv4Peer,
) -> Option<Ipv4Peer> {
    let remote = remote_address?;
    let ours = nat.addresses.as_deref()?;
    (nat.is_settled() && ours.contains(remote)).then(|| peer_address.clone())
}

/// One PEER_HOLEPUNCH round (JS 516-639). `analyze` and TRY_LATER are left out: the puzzle NAT is sampled once in `setup`.
pub(super) fn round(
    st: &mut PunchState,
    msg: &HolepunchMessage,
    from: &Ipv4Peer,
    peer_address: &Ipv4Peer,
    relays: &[RelayInfo],
    firewall: u64,
    filter: Option<&HolepunchFilter>,
) -> Round {
    let Some((remote, token)) = st.open(&msg.payload, &peer_address.host) else {
        return Round::Silent;
    };
    if remote.error != ERROR_NONE {
        return st.abort(remote.round);
    }
    let is_relay = relays.iter().any(|r| r.relay_address == *from);
    let echoed = is_relay && remote.remote_token == Some(token);
    st.observe(&remote, echoed.then_some(peer_address.host.as_str()));
    st.fast_ping(remote.remote_address.as_ref(), peer_address);
    if st.vetoed(&remote, filter) {
        return st.abort(remote.round);
    }
    let started = st.start_punch(remote.punching);
    let addresses = st.punch_addrs.clone();
    let payload = round_payload(
        ERROR_NONE,
        firewall,
        st.round,
        st.punching,
        addresses,
        is_relay.then_some(token),
        remote.token,
    );
    match st.seal(&payload) {
        Some(bytes) => Round::Reply {
            bytes,
            punching: started,
        },
        None => Round::Silent,
    }
}

#[cfg(test)]
#[path = "punch_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "punch_filter_tests.rs"]
mod filter_tests;

#[cfg(all(test, unix))]
#[path = "punch_life_tests.rs"]
mod life_tests;

#[cfg(all(test, unix))]
#[path = "punch_rebind_tests.rs"]
mod rebind_tests;
