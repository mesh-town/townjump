// JS: holesail/src/index.js (the Holesail class), holesail/src/bin/holesail.mjs (CLI).
// The `Holesail` facade (facade.rs) and the CLI (cli/); the network only through the townjump-dht traits, only main.rs plugs in the adapter.
pub mod cli;
pub mod facade;

pub use cli::{CliIo, run};
pub use facade::{Holesail, HolesailError, HolesailInfo, HolesailOpts};
