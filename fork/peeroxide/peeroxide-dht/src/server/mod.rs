// JS: hyperdht/lib/server.js
//! DHT server: `listen`, incoming connections and the firewall callback.
//!
//! Which file does what:
//! - `opts` — `ServerOpts`, `Firewall`, `FirewallInput` and the panic-safe `rejects`;
//! - `table` — deduplication of handshakes by Noise bytes, slots and their deadlines (no async);
//! - `reply` — pure builders of the Noise payload, holepunch round payloads and reply messages;
//! - `plan` — dial the client, wait for it, or hole-punch;
//! - `handshake` — one PEER_HANDSHAKE from the Noise message to our reply;
//! - `punch` — the puzzle (`Holepuncher`) and the PEER_HOLEPUNCH rounds;
//! - `conn` — a connection in the making (`Pending`), `dial` and `finish`;
//! - `actor_relay` — the actor's side of connections served over a relay;
//! - `actor` — the single task with the table and the event loop;
//! - `freeze` — the actor's side of the node's `suspend`/`resume`;
//! - `keepalive` — pings the relays of the announcement every 3 s, re-announces early or every ~5 min;
//! - `handle` — `Server`: `new`, `listen`, `close`;
//! - `test_net` (tests only) — local cluster, raw client handshake, echo.

mod actor;
mod actor_relay;
mod conn;
#[cfg(test)]
mod fanout_tests;
mod freeze;
#[cfg(test)]
mod freeze_tests;
mod handle;
mod handshake;
mod keepalive;
#[cfg(test)]
mod lan_dial_tests;
mod opts;
mod plan;
mod punch;
mod relay;
#[cfg(test)]
mod relay_flow_tests;
#[cfg(test)]
mod relay_retry_tests;
#[cfg(test)]
mod relay_upgrade_tests;
mod reply;
mod reply_build;
#[cfg(test)]
mod reuse_live_tests;
mod table;
#[cfg(test)]
mod test_net;
#[cfg(test)]
mod withdraw_relays_tests;
#[cfg(test)]
mod withdraw_time_tests;

pub use handle::Server;
pub use opts::{Firewall, FirewallInput, ServerOpts};
