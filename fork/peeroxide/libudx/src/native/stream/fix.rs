//! Sender fix switches via env `UDX_FIX`: comma-separated tokens `pace`, `ackstep`, `rack`, `legacy`.
//! By default (empty or no variable) `pace` and `ackstep` are built in — the C udx behavior; RACK by send order — always.
//! `legacy` — emergency fallback to the previous behavior (timer-based pacer): `pace` and `ackstep` are off.
//! Tokens `pace`, `ackstep` and `rack` are valid and change nothing (accepted so that existing scripts keep working).
//! `ackstep` (send step inline after an ACK packet) works only together with `pace`.
//! Unknown tokens are ignored.
use std::sync::OnceLock;

/// What `UDX_FIX` enables.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Mode {
    /// The pacer is refilled on every send attempt before the window check (C `stream_may_send`, udx.c:632-646).
    pub(super) pace: bool,
    /// A send step right after handling an ACK packet (C `send_packets` at the end of `process_packet`, udx.c:1793); only together with `pace`.
    pub(super) ackstep: bool,
}

/// Parsing of the `UDX_FIX` value.
pub(super) fn parse(value: &str) -> Mode {
    let mut mode = Mode {
        pace: true,
        ackstep: true,
    };
    for token in value.split(',').map(str::trim) {
        // Emergency fallback: timer-based pacer and no step after ACK.
        if token == "legacy" {
            mode.pace = false;
            mode.ackstep = false;
        }
    }
    mode
}

/// Mode from the environment; read once.
pub(super) fn mode() -> Mode {
    static MODE: OnceLock<Mode> = OnceLock::new();
    *MODE.get_or_init(|| parse(&std::env::var("UDX_FIX").unwrap_or_default()))
}

#[cfg(test)]
#[path = "fix_tests.rs"]
mod tests;
