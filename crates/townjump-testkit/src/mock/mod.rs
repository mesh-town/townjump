// JS: testvectors/holesail-cli/mock-dht.cjs. DHT mock: call trace, in-memory network, settings.
// config, events, trace - settings, events and their recording; factory, dht, server, shared, stub - in-memory network;
// knobs - gates and injected errors for tests. The mock has no relay: `relay_through` in the options is ignored.
pub mod config;
mod dht;
pub mod events;
mod factory;
mod knobs;
mod server;
mod shared;
mod stub;
pub mod trace;

#[cfg(test)]
mod matrix_tests;
#[cfg(test)]
mod test_util;

pub use config::{MockConfig, RecordFor};
pub use dht::MockDht;
pub use factory::MockFactory;
pub use server::MockServer;
pub use trace::{MockTrace, strip_untracked};
