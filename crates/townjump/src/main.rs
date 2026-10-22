// JS: holesail/src/bin/holesail.mjs (entry point). The only place with `#[tokio::main]` and the fork adapter.
// All behavior is in `townjump::cli::run`.
use townjump::{CliIo, run};
use townjump_dht_peeroxide::PeeroxideFactory;
use tracing_subscriber::EnvFilter;

/// Installs the global `tracing` subscriber from `RUST_LOG`; `true` — the subscriber was installed.
fn init_tracing() -> bool {
    init_tracing_with(EnvFilter::try_from_default_env().ok())
}

/// Fork records (peeroxide-dht, libudx) are visible only when a filter is set:
/// without it (`None`) no subscriber is installed and the binary output does not change by a single byte.
fn init_tracing_with(filter: Option<EnvFilter>) -> bool {
    let Some(filter) = filter else { return false };
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(true)
        .try_init()
        .is_ok()
}

#[tokio::main]
async fn main() {
    townjump::cli::fd_limit::raise_nofile_limit();
    init_tracing();
    let argv = std::env::args().skip(1).collect();
    std::process::exit(run(argv, CliIo::stdio(), PeeroxideFactory::from_env()).await);
}

#[cfg(test)]
mod main_tracing_tests;
