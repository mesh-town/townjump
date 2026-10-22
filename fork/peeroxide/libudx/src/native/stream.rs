// JS: libudx/src/udx.c
//! UDX stream (reliable delivery over UDP): entry point of the `stream/` directory. What is in which file:
//! - `consts.rs` — window, MTU, RTO, SACK and RTT limit constants;
//! - `types.rs` — incoming packet, events, sent/received records, aliases;
//! - `inner.rs` — `StreamInner` state and its base methods (terminate, inflight, send_idle);
//! - `mtu.rs` — MTU selection (`MtuState`, probe, padding removal);
//! - `outgoing.rs` — outgoing path: prepare_write/prepare_end, sent bookkeeping, packet assembly;
//! - `loss.rs` — RTT, cumulative ACK, loss marking (RACK, RTO); retransmission is in `retransmit.rs`;
//! - `retransmit.rs` — retransmissions through the window and pacing gates, ahead of new data;
//! - `rack.rs` — RACK clock (time and RTT of the last delivery, min RTT);
//! - `receive.rs` — data receive, SACK ranges, read events;
//! - `handle.rs` — public `UdxStream` handle (connect/write/read/shutdown/relay, Drop);
//! - `firewall.rs` — first-packet check and the `set_firewall_hook` hook;
//! - `destroy.rs` — `UdxStream::destroy`;
//! - `processor_task.rs` — processor task start and the write guard in the socket table;
//! - `incoming.rs` — incoming packet processor `process_incoming`;
//! - `send_batch.rs` — one sender wakeup (window, pacing, MTU probe);
//! - `send_step.rs` — sender loop step: batch to the socket, timer re-arm (RTO, ZWP, pacing);
//! - `control.rs` — `StreamControl` (route change and closing after `into_async_stream`);
//! - `path.rs` — MESSAGE, `await_direct`, the path hook (vetting packets from a foreign source);
//! - `upgrade.rs` — pending route change, closing, `connect` that moves a stream with a firewall hook;
//! - `stats.rs` — `UdxStats` snapshot for diagnostics (`stats()`);
//! - `sent_map.rs` — outgoing map with SACK/loss counters;
//! - `send_queue.rs` — write queue with a byte sum;
//! - `acks.rs`, `end.rs`, `init.rs`, `sack.rs`, `seq.rs`, `timers.rs`, `wire.rs` — ACK parsing, END, start, SACK, sequence numbers, timers, send.
use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use tokio::sync::{mpsc, oneshot, watch};

use super::congestion::CaState;
use super::header::{
    FLAG_DATA, FLAG_DESTROY, FLAG_END, FLAG_HEARTBEAT, FLAG_MESSAGE, FLAG_SACK, HEADER_SIZE,
    Header, SackRange,
};
use crate::error::{Result, UdxError};
// The `super::socket::…`, `super::congestion::…`, `super::async_stream::…` paths in the files of that directory go through these names.
use super::{async_stream, congestion, socket};

mod acks;
mod consts;
mod control;
#[cfg(test)]
mod delivered_once_tests;
mod destroy;
pub(in crate::native) mod diag;
#[cfg(test)]
mod diag_tests;
mod end;
#[cfg(test)]
mod f1_tests;
mod firewall;
#[cfg(test)]
mod firewall_gate_tests;
mod fix;
mod handle;
mod incoming;
mod init;
mod inner;
mod link_mtu;
#[cfg(test)]
mod link_mtu_tests;
mod loss;
mod mtu;
#[cfg(test)]
mod mtu_tests;
mod outgoing;
mod path;
#[cfg(test)]
mod probe_loss_tests;
mod processor_task;
mod rack;
mod receive;
#[cfg(test)]
mod recovery_tests;
#[cfg(test)]
mod relay_leak_tests;
mod relay_path;
mod retransmit;
mod sack;
mod send_batch;
mod send_queue;
mod send_step;
mod sent_map;
mod seq;
mod stats;
#[cfg(test)]
mod table_cleanup_tests;
#[cfg(test)]
mod test_support;
mod timers;
mod types;
mod upgrade;
#[cfg(test)]
mod upgrade_path_tests;
#[cfg(test)]
mod upgrade_tests;
mod wire;
#[cfg(test)]
mod wrap_tests;
mod write_budget;

pub(crate) use consts::{DEFAULT_RWND, WRITE_HIGH_WATERMARK};
pub use control::{RemoteChanged, StreamControl, StreamPeer};
pub use handle::UdxStream;
pub(crate) use inner::StreamInner;
pub(crate) use outgoing::build_data_packet;
pub use stats::UdxStats;
pub(crate) use types::{IncomingPacket, PendingWrite, StreamEvent, StreamNotify};

use consts::*;
use firewall::is_plausible_first_packet;
use incoming::process_incoming;
use mtu::{MtuState, strip_probe_padding};
use outgoing::send_data_packets;
use processor_task::TableEntry;
use sack::incoming_sacks;
use send_queue::SendQueue;
use sent_map::SentMap;
use seq::{Seq, seq_after, seq_diff};
use types::*;
use upgrade::RemoteChange;
use wire::send_ack;
