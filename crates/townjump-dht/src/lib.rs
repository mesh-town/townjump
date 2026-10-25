// JS: `hyperdht` calls from holesail-server/index.js and holesail-client/index.js (createServer, connect, mutablePut/Get).
// Network traits: server, client and CLI know only this crate; implementations are the fork adapter and `MockFactory` from testkit.
pub mod duplex;
pub mod error;
pub mod incoming;
pub mod mode;
pub mod traits;
pub mod types;

pub use duplex::Duplex;
pub use error::DhtError;
pub use incoming::Incoming;
pub use mode::NodeMode;
pub use townjump_keys::KeyPair;
pub use traits::{Dht, DhtFactory, DhtServer};
pub use types::{ConnectOpts, DhtConnection, Firewall, MutableRecord, RttProbe, ServerOpts};
