// JS: holesail-client/index.js (HolesailClient, ping); behaviour is in docs/spec/holesail/{client,dht-records}.md.
// Client: reads the server record, listens on a local port, opens a new DHT connection for each connection.
pub mod args;
pub mod client;
pub mod connect;
pub mod error;
pub mod info;
pub mod ping;

pub use args::ConnectArgs;
pub use client::HolesailClient;
pub use error::ClientError;
pub use info::ClientInfo;
pub use ping::{LookupRecord, ping};

#[cfg(test)]
#[path = "support_tests.rs"]
mod support;
