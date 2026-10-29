//! Client relay options: which relay to ask for (`RelayThrough`) and the holepunch veto (`HolepunchFilter`).
// JS: hyperdht/lib/connect.js:53 (relayThrough), :98-106 (holepunch)

use std::fmt;
use std::sync::Arc;

use super::super::types::HyperDhtError;
use crate::holepuncher::Holepuncher;
use crate::messages::Ipv4Peer;

type SelectFn = Arc<dyn Fn() -> Option<[u8; 32]> + Send + Sync>;

/// Which blind-relay node to route a connection through (JS `opts.relayThrough`).
#[derive(Clone)]
#[non_exhaustive]
pub enum RelayThrough {
    /// One fixed relay.
    Key([u8; 32]),
    /// Keys tried in turn from a random start: the next key only after our own relay
    /// aborted the pairing (an extension over JS, which picks one key per connect).
    List(Vec<[u8; 32]>),
    /// Ask a function on every connect; `None` means no relay.
    Select(SelectFn),
}

impl RelayThrough {
    /// The relay key for this connect, or `None` for no relay.
    pub fn select(&self) -> Option<[u8; 32]> {
        match self {
            Self::Key(k) => Some(*k),
            Self::List(l) if l.is_empty() => None,
            Self::List(l) => Some(l[rand::random_range(0..l.len())]),
            Self::Select(f) => f(),
        }
    }

    /// The relay keys of one connect in the order they are tried: `Key` — itself; `List` — every
    /// distinct key once, round from `start % len`; `Select` — one call of the function.
    pub(crate) fn candidates(&self, start: usize) -> Vec<[u8; 32]> {
        match self {
            Self::Key(k) => vec![*k],
            Self::List(l) if l.is_empty() => Vec::new(),
            Self::List(l) => {
                let mut out: Vec<[u8; 32]> = Vec::with_capacity(l.len());
                let first = start % l.len();
                for k in l[first..].iter().chain(&l[..first]) {
                    if !out.contains(k) {
                        out.push(*k);
                    }
                }
                out
            }
            Self::Select(f) => f().into_iter().collect(),
        }
    }
}

impl fmt::Debug for RelayThrough {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Key(k) => f.debug_tuple("Key").field(k).finish(),
            Self::List(l) => f.debug_tuple("List").field(&l.len()).finish(),
            Self::Select(_) => f.write_str("Select(..)"),
        }
    }
}

/// What the holepunch filter sees once the peer has answered the first probe.
#[non_exhaustive]
pub struct HolepunchInput<'a> {
    /// The peer's firewall mode.
    pub remote_firewall: u64,
    /// Our own firewall mode.
    pub local_firewall: u64,
    /// Addresses the peer announced.
    pub remote_addresses: &'a [Ipv4Peer],
    /// Addresses we announced (empty while unknown).
    pub local_addresses: &'a [Ipv4Peer],
}

type FilterFn = Arc<dyn Fn(&HolepunchInput<'_>) -> bool + Send + Sync>;

/// Veto on holepunching (JS `opts.holepunch`): `false` aborts the punch.
#[derive(Clone)]
pub struct HolepunchFilter(FilterFn);

impl HolepunchFilter {
    /// Wrap a predicate; `true` lets the punch run.
    pub fn new(f: impl Fn(&HolepunchInput<'_>) -> bool + Send + Sync + 'static) -> Self {
        Self(Arc::new(f))
    }

    /// Whether the punch may go on for `input`.
    pub fn allows(&self, input: &HolepunchInput<'_>) -> bool {
        (self.0)(input)
    }
}

impl fmt::Debug for HolepunchFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("HolepunchFilter(..)")
    }
}

/// Apply the filter to a puncher that has seen the peer's first answer; no filter allows.
fn punch_allowed(filter: Option<&HolepunchFilter>, p: &Holepuncher) -> bool {
    let Some(filter) = filter else { return true };
    let remote: Vec<Ipv4Peer> = p
        .remote_addresses
        .iter()
        .map(|a| Ipv4Peer {
            host: a.host.clone(),
            port: a.port,
        })
        .collect();
    let local = p.nat.addresses.as_deref().unwrap_or(&[]);
    filter.allows(&HolepunchInput {
        remote_firewall: p.remote_firewall,
        local_firewall: p.nat.firewall,
        remote_addresses: &remote,
        local_addresses: local,
    })
}

/// Veto point after the first probe answer: a refusal destroys the puncher (JS `abort()`) and fails the punch.
pub(crate) fn gate_punch(
    filter: Option<&HolepunchFilter>,
    p: &mut Holepuncher,
) -> Result<(), HyperDhtError> {
    if punch_allowed(filter, p) {
        return Ok(());
    }
    p.destroy();
    Err(HyperDhtError::HolepunchAborted)
}
