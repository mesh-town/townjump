// JS: hyperdht/lib/server.js
//! The server's single task: it owns the table of handshakes and handles
//! DHT events, commands from `Server`, and "client arrived" notices one by one.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use libudx::UdxRuntime;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::sync::oneshot;
use tokio::time::MissedTickBehavior;

use super::actor_relay as arelay;
use super::conn::{self, Pending};
use super::handshake::{self, Hello};
use super::opts::ServerOpts;
use super::punch::{self, Round};
use super::relay::Relayed;
use super::reply::encode_holepunch;
use super::table::Table;
use crate::hyperdht::{
    Freeze, HyperDhtError, HyperDhtHandle, KeyPair, PeerConnection, RelayPaired, ServerEvent,
};
use crate::hyperdht_messages::{FIREWALL_UNKNOWN, HolepunchMessage, RelayInfo};
use crate::messages::Ipv4Peer;

/// How often deadlines of pending connections are checked.
const TICK: Duration = Duration::from_secs(1);

/// Orders from `Server` to the actor.
pub(super) enum Command {
    /// Start serving `key_pair` under `target` (`hash(public_key)`).
    Listen(KeyPair, [u8; 32]),
    /// The relays of the latest announcement (our address as the relay sees it).
    SetRelays(Vec<RelayInfo>),
    /// The relays to keep alive, `None` while suspended (`keepalive` asks every round).
    Relays(oneshot::Sender<Option<Vec<RelayInfo>>>),
    /// Stop serving and drop every pending connection.
    Close,
}

/// Notices from the UDX firewall hooks to the actor.
pub(super) enum Internal {
    /// The first packet of the client of slot `id` came from `addr`.
    Arrived { id: u64, addr: SocketAddr },
    /// The relay pairing of slot `id` ended.
    RelayPaired {
        id: u64,
        paired: Result<RelayPaired, HyperDhtError>,
    },
    /// The relayed connection of slot `id` went direct or closed.
    RelayDone { id: u64 },
}

/// What the server listens for.
pub(super) struct Listening {
    pub(super) key_pair: KeyPair,
    pub(super) target: [u8; 32],
    pub(super) relays: Vec<RelayInfo>,
}

/// The actor's state.
pub(super) struct State {
    pub(super) dht: HyperDhtHandle,
    pub(super) rt: UdxRuntime,
    pub(super) opts: ServerOpts,
    pub(super) incoming: mpsc::Sender<PeerConnection>,
    pub(super) table: Table<Pending>,
    pub(super) listening: Option<Listening>,
    /// JS `server.suspended`: handshakes get no answer.
    pub(super) suspended: bool,
    /// Key pair for the connections to relays (JS `dht.defaultKeyPair`), made once.
    pub(super) relay_key: KeyPair,
    /// Puzzles of connections served over a relay, by slot id.
    pub(super) relayed: HashMap<u64, Relayed>,
}

impl State {
    /// Idle state: not listening, empty table.
    pub(super) fn new(
        dht: HyperDhtHandle,
        rt: UdxRuntime,
        opts: ServerOpts,
        incoming: mpsc::Sender<PeerConnection>,
    ) -> Self {
        let relay_key = opts
            .relay_key_pair
            .clone()
            .unwrap_or_else(KeyPair::generate);
        Self {
            dht,
            rt,
            opts,
            incoming,
            table: Table::new(),
            listening: None,
            suspended: false,
            relay_key,
            relayed: HashMap::new(),
        }
    }

    /// Relays to advertise (empty when not listening).
    pub(super) fn relays(&self) -> Vec<RelayInfo> {
        self.listening
            .as_ref()
            .map(|l| l.relays.clone())
            .unwrap_or_default()
    }

    fn stop(&mut self) {
        self.listening = None;
        self.table
            .clear_all()
            .into_iter()
            .for_each(Pending::destroy);
        arelay::stop_relayed(self);
    }
}

/// The relay nodes' addresses, in order.
pub(super) fn relay_addresses(relays: &[RelayInfo]) -> Vec<Ipv4Peer> {
    relays.iter().map(|r| r.relay_address.clone()).collect()
}

/// The event loop; it ends when the command channel is closed.
pub(super) async fn run(
    mut st: State,
    mut events: UnboundedReceiver<ServerEvent>,
    mut cmds: UnboundedReceiver<Command>,
    internal: (UnboundedSender<Internal>, UnboundedReceiver<Internal>),
    mut freeze: UnboundedReceiver<Freeze>,
) {
    let (internal_tx, mut internal_rx) = internal;
    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            Some(event) = events.recv() => on_event(&mut st, event, &internal_tx).await,
            cmd = cmds.recv() => match cmd {
                Some(cmd) => on_command(&mut st, cmd),
                None => break,
            },
            Some(notice) = internal_rx.recv() => on_internal(&mut st, notice, &internal_tx),
            Some(order) = freeze.recv() => super::freeze::on_freeze(&mut st, order).await,
            _ = tick.tick() => on_tick(&mut st),
        }
    }
    st.stop();
}

fn on_command(st: &mut State, cmd: Command) {
    match cmd {
        Command::Listen(key_pair, target) => {
            st.listening = Some(Listening {
                key_pair,
                target,
                relays: vec![],
            });
        }
        Command::SetRelays(relays) => {
            if let Some(listening) = st.listening.as_mut() {
                listening.relays = relays;
            }
        }
        Command::Relays(reply) => {
            let _ = reply.send((!st.suspended).then(|| st.relays()));
        }
        Command::Close => st.stop(),
    }
}

async fn on_event(st: &mut State, event: ServerEvent, internal: &UnboundedSender<Internal>) {
    match event {
        ServerEvent::PeerHandshake {
            msg,
            from,
            peer_address,
            target,
            reply_tx,
            route,
        } => {
            let hello = Hello {
                msg,
                from,
                peer_address,
                target,
                route,
            };
            handshake::on_handshake(st, hello, reply_tx, internal).await;
        }
        ServerEvent::PeerHolepunch {
            msg,
            from,
            peer_address,
            reply_tx,
            ..
        } => {
            let bytes = on_holepunch(st, &msg, &from, &peer_address).await;
            let _ = reply_tx.send(bytes);
        }
    }
}

fn on_tick(st: &mut State) {
    let now = Instant::now();
    st.table.expire(now).into_iter().for_each(Pending::destroy);
    arelay::expire_relayed(st, now);
}

fn on_internal(st: &mut State, notice: Internal, internal: &UnboundedSender<Internal>) {
    match notice {
        Internal::Arrived { id, addr } => on_arrived(st, id, addr),
        Internal::RelayPaired { id, paired } => arelay::on_relay_paired(st, id, paired, internal),
        Internal::RelayDone { id } => arelay::on_relay_done(st, id),
    }
}

/// The client reached us: the connection leaves the slot and is finished in its own task.
fn on_arrived(st: &mut State, id: u64, addr: SocketAddr) {
    let Some(mut pending) = st.table.take(id) else {
        return;
    };
    pending.relay = None;
    st.table.clear_later(id, Instant::now());
    tokio::spawn(conn::finish(pending, addr, st.incoming.clone()));
}

/// One PEER_HOLEPUNCH round for the slot `msg.id`; `None` = no answer.
async fn on_holepunch(
    st: &mut State,
    msg: &HolepunchMessage,
    from: &Ipv4Peer,
    peer_address: &Ipv4Peer,
) -> Option<Vec<u8>> {
    let firewall = st.dht.firewalled().await.unwrap_or(FIREWALL_UNKNOWN);
    let relays = st.relays();
    let filter = st.opts.holepunch.clone();
    let punch = match st.table.get_mut(msg.id) {
        Some(pending) => pending.punch.as_mut()?,
        None => &mut st.relayed.get_mut(&msg.id)?.punch,
    };
    let round = punch::round(
        punch,
        msg,
        from,
        peer_address,
        &relays,
        firewall,
        filter.as_ref(),
    );
    let now = Instant::now();
    match round {
        Round::Silent => None,
        Round::Reply { bytes, punching } => {
            if punching {
                arelay::extend_slot(st, msg.id, now);
            }
            encode_holepunch(msg, peer_address, bytes)
        }
        Round::Abort(bytes) => {
            arelay::abort_slot(st, msg.id, now);
            encode_holepunch(msg, peer_address, bytes)
        }
    }
}
