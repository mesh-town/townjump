//! IO layer for the DHT-RPC protocol.
//!
//! Faithful Rust port of the Node.js dht-rpc IO layer.
//! The [`crate::io::Io`] struct is driven by the caller from a
//! `tokio::select!` loop.
//!
//! Files (`io/`): `types` — errors, config, events, request records; `window` — congestion
//! window; `state` — the `Io` struct, `bind`, sockets, `destroy`; `event_loop` — `recv`, `drain`,
//! timeouts; `requests` — outgoing requests and retransmits; `cancel` — dropping requests by owner;
//! `replies` — replies and `reply_to`; `tokens` — token secrets; `relay` — relayed sends; `datagram` — inbound datagram dispatch.
//! Suspend/resume — `io_suspend.rs`; tests — `io_tests.rs`.

#![allow(missing_docs)]

// JS: dht-rpc/lib/io.js suspend/resume
#[path = "io_suspend.rs"]
mod suspend;

mod cancel;
mod datagram;
mod event_loop;
mod relay;
mod replies;
mod requests;
mod state;
mod tokens;
mod types;
mod window;

pub use state::Io;
pub(crate) use types::Owner;
pub(crate) use types::ReplyContext;
pub(crate) use types::RetryEvent;
pub use types::{
    IncomingRequest, IoConfig, IoError, IoEvent, IoResult, IoStats, RequestParams, ResolvedRequest,
    SocketKind, TimeoutEvent, WireCounters,
};
pub use window::CongestionWindow;

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "io_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "io_retry_tests.rs"]
mod retry_tests;

#[cfg(test)]
#[path = "io_cancel_tests.rs"]
mod cancel_tests;

#[cfg(test)]
#[path = "io_via_tests.rs"]
mod via_tests;
