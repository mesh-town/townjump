//! `handshake-trace` journal lines of a connect: log only, no effect on behaviour.
//! Target is `…::connect`, so the filters that already enable `connect=debug` show them.

use std::fmt;

use tokio::time::Instant;

use crate::messages::Ipv4Peer;

/// `host:port` of the relay, `-` when the step has none.
struct RelayFmt<'a>(Option<&'a Ipv4Peer>);

impl fmt::Display for RelayFmt<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(p) => write!(f, "{}:{}", p.host, p.port),
            None => f.write_str("-"),
        }
    }
}

/// One trace line; `t0` is the start of the enclosing step (formatting is lazy, off below debug).
pub(super) fn trace(step: &str, relay: Option<&Ipv4Peer>, t0: Instant) {
    tracing::debug!(
        target: "peeroxide_dht::hyperdht::connect",
        "handshake-trace: {step} relay={} t_ms={}",
        RelayFmt(relay),
        t0.elapsed().as_millis()
    );
}
