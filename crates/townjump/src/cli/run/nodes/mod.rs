// JS: none — a port extension; holesail JS starts every node with an empty table.
// Saved DHT nodes: file format (file.rs) and the `--save-nodes*` flags (opts.rs).

mod file;
mod opts;
mod saver;

pub(crate) use file::{MAX_NODES, load, save, unix_now};
pub(crate) use opts::{NodesOpts, nodes_opts};
pub(crate) use saver::{Saver, seed, start};
