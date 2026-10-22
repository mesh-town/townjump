// JS: holesail/src/bin/holesail.mjs (top level: parsing, checks, dispatcher, `--log`, `--lookup`).
// See also: argv.rs → checks.rs → help.rs/banner.rs; server and client go through the facade.rs facade.
use super::argv::parse_argv;
use super::checks::validate_input;
use super::help::{help_text, version_text};
use std::sync::{Arc, Mutex};
use townjump_dht::DhtFactory;
use townjump_jsval::JsValue;
use townjump_logger::SharedOut;

mod args;
mod filemanager;
mod interrupt;
mod lookup;
mod nodes;
mod serve;

pub use interrupt::{Interrupt, manual_interrupt, os_interrupt};

use args::Mode;
use nodes::{NodesOpts, nodes_opts, seed};

/// Randomness source: fills a buffer (`crypto.randomBytes`).
pub type RandomFill = Box<dyn FnMut(&mut [u8]) + Send>;

/// CLI I/O: streams, randomness source and stop signals (tests substitute buffers, `000102…1f`, `manual_interrupt`).
pub struct CliIo {
    pub stdout: SharedOut,
    pub stderr: SharedOut,
    pub random: RandomFill,
    /// Subscription to SIGINT/SIGTERM; the `--filemanager` branch and live/connect with `--save-nodes` call it, after the banner.
    pub interrupt: Interrupt,
}

impl CliIo {
    /// Real stdout, stderr and system randomness.
    pub fn stdio() -> CliIo {
        CliIo {
            stdout: Arc::new(Mutex::new(std::io::stdout())),
            stderr: Arc::new(Mutex::new(std::io::stderr())),
            random: Box::new(|buf: &mut [u8]| {
                getrandom::fill(buf).expect("system randomness source unavailable");
            }),
            interrupt: os_interrupt(),
        }
    }
}

/// Writes the text to a stream whole and flushes the buffer; write errors (a closed pipe) are silently lost, like `console.log`.
pub(crate) fn emit(out: &SharedOut, text: &str) {
    let mut out = out.lock().unwrap_or_else(|e| e.into_inner());
    let _ = out.write_all(text.as_bytes());
    let _ = out.flush();
}

/// Exit code. Server and client exit on `Fatal` (or, with `--save-nodes`, a signal: 130); help, version, `--lookup` — 0; input check — 2.
pub async fn run<F: DhtFactory>(argv: Vec<String>, io: CliIo, factory: F) -> i32 {
    let args = match parse_argv(argv) {
        Ok(args) => args,
        Err(error) => return fail(&io, &error.to_string()),
    };
    let checked = validate_input(&args, &|path| std::path::Path::new(path).exists());
    emit(&io.stdout, &checked.stdout);
    if let Some(code) = checked.exit {
        return code;
    }
    dispatch(&args, io, factory).await
}

/// Unhandled error: first line of the message to stderr, code 1.
pub(crate) fn fail(io: &CliIo, message: &str) -> i32 {
    emit(&io.stderr, &format!("{message}\n"));
    1
}

/// The `holesail.mjs` dispatcher: the first branch that fires.
async fn dispatch<F: DhtFactory>(args: &JsValue, io: CliIo, mut factory: F) -> i32 {
    let mode = Mode::of(args);
    let nodes = match saved_nodes(args, &mode, &mut factory) {
        Ok(nodes) => nodes,
        Err(message) => return fail(&io, &message),
    };
    match mode {
        Mode::Help => print_help(&io, args),
        Mode::Version => print_text(&io, &version_text()),
        Mode::Live => serve::server(args, io, factory, nodes).await,
        Mode::Connect => serve::client(args, io, factory, nodes).await,
        Mode::Filemanager => filemanager::run(args, io, factory, nodes).await,
        Mode::Lookup => lookup::run(args, &io, &factory).await,
        Mode::Fallback => print_help(&io, args),
    }
}

/// `--save-nodes*` flags for the modes that own a DHT node; feeds the saved nodes to the factory.
fn saved_nodes<F: DhtFactory>(
    args: &JsValue,
    mode: &Mode,
    factory: &mut F,
) -> Result<Option<NodesOpts>, String> {
    if !matches!(
        mode,
        Mode::Live | Mode::Connect | Mode::Filemanager | Mode::Lookup
    ) {
        return Ok(None);
    }
    let nodes = nodes_opts(args)?;
    if let Some(opts) = &nodes {
        seed(factory, opts);
    }
    Ok(nodes)
}

fn print_help(io: &CliIo, args: &JsValue) -> i32 {
    let topic = args.get("help").ok().flatten();
    print_text(io, &help_text(topic))
}

fn print_text(io: &CliIo, text: &str) -> i32 {
    emit(&io.stdout, text);
    0
}

#[cfg(test)]
#[path = "run_tests.rs"]
mod tests;
