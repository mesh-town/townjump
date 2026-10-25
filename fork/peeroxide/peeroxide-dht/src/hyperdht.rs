//! HyperDHT node and client/server API, port of Node.js `hyperdht`.
//!
//! Files (`hyperdht/`): `types`, `keypair`, `connection`, `config` — errors, results, keys, options;
//! `handle` (+ `lookup`, `announce`, `records`) — the `HyperDhtHandle` API; `connect*` — client connect:
//! search, `connect_finish` (direct or holepunch), `connect_holepunch`, `connect_fast_open` (fast mode), `connect_relayed`; `server_run`,
//! `server_passive` — the server actor; `node*` — `spawn` and the PEER_HANDSHAKE / PEER_HOLEPUNCH
//! handlers; `stream` — stream ids and `establish_stream*`. Suspend/resume — `hyperdht_suspend.rs`;
//! tests — `hyperdht_*tests.rs`.
#![deny(clippy::all)]

#[path = "hyperdht_suspend.rs"]
mod suspend;
pub(crate) use suspend::Freeze;
mod announce;
mod config;
mod connect;
mod connect_error;
pub(crate) mod connect_fast_open;
mod connect_final_round;
mod connect_finish;
mod connect_flow;
mod connect_holders;
mod connect_holepunch;
mod connect_lan;
mod connect_payload;
mod connect_race;
mod connect_relayed;
mod connect_route;
mod connect_trace;
mod connection;
mod handle;
mod keypair;
mod lan_pick;
mod lan_probe;
mod lookup;
mod node;
mod node_handshake;
mod node_holepunch;
mod records;
mod relay;
mod relay_cache;
mod reuse;
mod server_passive;
mod server_run;
mod stream;
mod types;
mod unannounce;

use crate::rpc::DhtError;

#[cfg(test)]
pub(crate) use connect_finish::client_same_host;
pub(crate) use connect_finish::is_addr_private;
#[cfg(test)]
pub(crate) use connect_lan::{LAN_PROBE_DEADLINE, LanProbe, LanRewrite};
#[cfg(test)]
pub(crate) use lan_probe::LanProbes;
pub(crate) use relay::{AbortOnDrop, DirectSource, RelayPaired, Upgrade, pair_via_relay};
#[cfg(test)]
pub(crate) use reuse::LINGER;
pub(crate) use reuse::Reuse;

pub use config::{ConnectOpts, DEFAULT_BOOTSTRAP, HyperDhtConfig, ServerConfig};
pub use connect_finish::should_direct_connect;
pub use connection::{ConnectResult, PeerConnection};
pub use handle::HyperDhtHandle;
pub use keypair::KeyPair;
pub use node::spawn;
pub use relay::{HolepunchFilter, HolepunchInput, RelayPath, RelayThrough};
pub use server_passive::{PassiveHolepunchReply, build_passive_holepunch_reply};
pub use server_run::{ServerSession, run_server};
pub use stream::{alloc_stream_id, establish_stream, establish_stream_with_socket};
pub use types::{
    AnnounceResult, HyperDhtError, ImmutablePutResult, LookupResult, MutableGetResult,
    MutablePutResult, ServerEvent,
};

#[cfg(test)]
#[path = "hyperdht_mutable_latest_tests.rs"]
mod mutable_latest_tests;
#[cfg(test)]
#[path = "hyperdht_passive_tests.rs"]
mod passive_holepunch_helper_tests;
#[cfg(test)]
#[path = "hyperdht_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "hyperdht_topology_tests.rs"]
mod topology_tests;
