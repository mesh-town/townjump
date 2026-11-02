//! LAN discovery of townjump servers over IPv4 multicast.
//!
//! A client that knows a server's public key asks the local network for it
//! before (or while) the DHT is consulted. The key itself never goes on the
//! wire: packets carry only its hash, the `target`, which the DHT announces
//! publicly anyway.
//!
//! Packets (UDP, group `239.255.84.74:28474` by default, TTL 1):
//! * request: `"TJLD" + version(1) + nonce(8) + target(32)`, 45 bytes;
//! * reply: the same fields plus `port` (2 bytes, big-endian), 47 bytes,
//!   sent unicast to the asker. The server address is the source IP of the
//!   reply; only the port travels in the payload.
//!
//! Trailing bytes are ignored, foreign or malformed packets are dropped
//! silently. The crate has no DHT or crypto dependency: the caller supplies
//! the target as an opaque 32-byte value.

use std::net::Ipv4Addr;

pub mod codec;
pub mod interfaces;
mod membership;
pub mod prober;
pub mod responder;

pub use codec::{DEFAULT_GROUP, DEFAULT_PORT, Nonce, Target};
pub use interfaces::InterfaceSource;
pub use prober::{ProbeTiming, find};
pub use responder::Responder;

/// Multicast group and port shared by responders and probers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LanConfig {
    pub group: Ipv4Addr,
    pub port: u16,
}

impl Default for LanConfig {
    fn default() -> Self {
        Self {
            group: DEFAULT_GROUP,
            port: DEFAULT_PORT,
        }
    }
}

#[cfg(test)]
#[path = "membership_tests.rs"]
mod membership_tests;

#[cfg(test)]
#[path = "lan_tests.rs"]
mod lan_tests;
