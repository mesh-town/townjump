//! DHT-RPC node: request dispatch, replies, queries, routing-table upkeep and the user handle.
//!
//! Files (`rpc/`): `consts` — intervals, command ids; `types` — errors, config, params;
//! `requests` — `UserRequest`, standalone and deferred records; `command` — `DhtCommand`;
//! `handle`, `handle_relay`, `handle_state` — `DhtHandle` (requests, relay sends, state getters);
//! `session` — `Session` (scope that cancels its requests and queries);
//! `query_stream` — `QueryStream` (query whose value replies are polled as they arrive);
//! `node` — `DhtNode` struct, `run`, `spawn`; `node_events`, `node_responses`, `node_tick`,
//! `node_command`, `node_query`, `node_table` — `impl DhtNode` by theme.
//! Suspend/resume — `rpc_suspend.rs`; tests — `rpc_tests.rs`.

#![deny(clippy::all)]

// JS: dht-rpc/index.js suspend/resume
#[path = "rpc_suspend.rs"]
mod suspend;

mod adaptive;
mod command;
mod consts;
mod handle;
mod handle_mode;
mod handle_relay;
mod handle_state;
mod nat_check;
mod nat_window;
mod node;
mod node_adaptive;
mod node_command;
mod node_events;
mod node_nat;
mod node_query;
mod node_responses;
mod node_seeds;
mod node_table;
mod node_tick;
mod query_stream;
mod requests;
mod session;
mod types;

use command::DhtCommand;
use consts::OLD_NODE;
use node::DhtNode;
use requests::StandaloneRequest;

pub(crate) use consts::ERR_UNKNOWN_COMMAND;
pub use handle::DhtHandle;
pub use node::spawn;
pub(crate) use query_stream::QueryStream;
pub use requests::UserRequest;
pub use session::Session;
pub use types::{
    DhtConfig, DhtError, DhtMode, PingResponse, ResponseData, UserQueryParams, UserRequestParams,
};

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod node_test_support;

#[cfg(test)]
#[path = "rpc_session_tests.rs"]
mod session_tests;

#[cfg(test)]
mod via_tests;

#[cfg(test)]
#[path = "rpc_tests.rs"]
mod tests;
