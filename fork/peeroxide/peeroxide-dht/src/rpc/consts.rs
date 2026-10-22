// JS: dht-rpc/index.js
//! Timer intervals, tick limits, command ids and the reply-suppression sentinel of the RPC node.
//! Neighbours: `node*` files read them; `types` holds the public config.

use crate::messages::Command;

pub(super) const TICK_INTERVAL_MS: u64 = 5_000;
pub(super) const DRAIN_INTERVAL_MS: u64 = 750;
pub(super) const SLEEPING_INTERVAL_MS: u64 = 3 * TICK_INTERVAL_MS;
pub(super) const REFRESH_TICKS: u64 = 60;
pub(super) const RECENT_NODE: u64 = 12;
pub(super) const OLD_NODE: u64 = 360;
pub(super) const MAX_REPINGING: u32 = 3;
pub(super) const DOWN_HINTS_RATE_LIMIT: u32 = 50;
/// Ticks an ephemeral node waits before checking whether it can become persistent.
pub const STABLE_TICKS: u64 = 240;

pub(super) const CMD_PING: u64 = Command::Ping as u64;
pub(super) const CMD_PING_NAT: u64 = Command::PingNat as u64;
pub(super) const CMD_FIND_NODE: u64 = Command::FindNode as u64;
pub(super) const CMD_DOWN_HINT: u64 = Command::DownHint as u64;
pub(super) const CMD_DELAYED_PING: u64 = Command::DelayedPing as u64;

pub(crate) const ERR_UNKNOWN_COMMAND: u64 = 1;

/// Internal sentinel sent on `UserRequest::reply_tx` by [`UserRequest::release`]
/// to instruct the deferred-reply pipeline to suppress the auto-emitted REPLY
/// packet (the relay-forwarding paths consume the inbound REQUEST without
/// responding; the original requester's REPLY arrives via the tid-preserved
/// chain instead).
pub(super) const SUPPRESS_REPLY_SENTINEL: u64 = u64::MAX;
