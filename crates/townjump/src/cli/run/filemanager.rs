// JS: holesail/src/bin/holesail.mjs:79–122 (the `--filemanager` branch): Livefiles → Holesail → banner → goodbye.
// See also: interrupt.rs (signals), args.rs (options), serve.rs (the model); the townjump-filemanager crate.
use super::args::{flag, log_level, server_opts};
use super::interrupt::{SIGNAL_EXIT, signalled};
use super::nodes::{NodesOpts, Saver, start};
use super::{CliIo, emit};
use crate::cli::banner::filemanager_banner;
use crate::facade::{Holesail, HolesailOpts};
use std::future::Future;
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedReceiver;
use townjump_dht::{Dht, DhtFactory};
use townjump_filemanager::{FatalHook, Filemanager, FilemanagerOpts};
use townjump_jsval::JsValue;
use townjump_logger::{Fatal, Logger, SharedOut};

pub(super) async fn run<F: DhtFactory>(
    args: &JsValue,
    io: CliIo,
    factory: F,
    nodes: Option<NodesOpts>,
) -> i32 {
    let CliIo {
        stdout,
        stderr,
        mut random,
        interrupt,
    } = io;
    let logger = Logger::new(log_level(args), stdout.clone(), stderr.clone());
    let (fatal, fatals) = Fatal::channel();
    let mut files = match Filemanager::new(files_opts(args, hook(&fatal))) {
        Ok(files) => files,
        Err(error) => return die(&stderr, &error.to_string()),
    };
    if let Err(error) = files.ready().await {
        return die(&stderr, &error.to_string());
    }
    let holesail = Holesail::new(factory, holesail_opts(args), logger, fatal, &mut random);
    let mut holesail = match holesail {
        Ok(holesail) => holesail,
        Err(error) => return stop_files(&mut files, &stderr, &error.to_string()).await,
    };
    if let Err(error) = holesail.ready().await {
        return stop_files(&mut files, &stderr, &error.to_string()).await;
    }
    emit(
        &stdout,
        &filemanager_banner(&holesail.info(), &files.info()),
    );
    let saver = nodes.and_then(|o| holesail.dht().map(|dht| start(dht, &o)));
    let signals = interrupt();
    let mut running = Running {
        holesail,
        files,
        saver,
    };
    wait_for_exit(&mut running, fatals, signals, &stderr).await
}

struct Running<F: DhtFactory> {
    holesail: Holesail<F>,
    files: Filemanager,
    saver: Option<Saver<F::Dht>>,
}

/// Until `Fatal` (code 1) or a signal (130); the table is saved first in both cases.
async fn wait_for_exit<F: DhtFactory>(
    running: &mut Running<F>,
    mut fatals: UnboundedReceiver<String>,
    mut signals: UnboundedReceiver<()>,
    stderr: &SharedOut,
) -> i32 {
    tokio::select! {
        biased;
        message = fatals.recv() => {
            finish(running.saver.take()).await;
            die(stderr, &message.unwrap_or_default())
        }
        () = signalled(&mut signals) => {
            let closing = async {
                finish(running.saver.take()).await;
                shutdown(&mut running.holesail, &mut running.files).await;
            };
            shutdown_on_signal(&mut signals, closing).await;
            SIGNAL_EXIT
        }
    }
}

/// The last table write, if saving is on.
async fn finish<D: Dht>(saver: Option<Saver<D>>) {
    if let Some(saver) = saver {
        saver.finish().await;
    }
}

/// A second signal during shutdown aborts it at once (in JS the signal itself finishes it off).
pub(super) async fn shutdown_on_signal<Fut: Future<Output = ()>>(
    signals: &mut UnboundedReceiver<()>,
    closing: Fut,
) {
    tokio::select! {
        biased;
        () = closing => {}
        () = signalled(signals) => {}
    }
}

/// Stop on a signal: the holesail facade, then the file server (like `goodbye` in holesail.mjs).
async fn shutdown<F: DhtFactory>(holesail: &mut Holesail<F>, files: &mut Filemanager) {
    let _ = holesail.close().await;
    files.close().await;
}

/// Error before the banner: the file server is already listening, we close it and exit with code 1.
async fn stop_files(files: &mut Filemanager, stderr: &SharedOut, message: &str) -> i32 {
    files.close().await;
    die(stderr, message)
}

/// Unhandled error: first line to stderr, code 1 (like `run::fail`).
fn die(stderr: &SharedOut, message: &str) -> i32 {
    emit(stderr, &format!("{message}\n"));
    1
}

fn hook(fatal: &Fatal) -> FatalHook {
    let fatal = fatal.clone();
    Arc::new(move |message| fatal.raise(message))
}

/// `--safe-mode`: a bare flag and any value enable the mode, except `false`/`0`/`no` (case-insensitive).
fn safe_mode_on(v: Option<&JsValue>) -> bool {
    match v {
        Some(JsValue::String(s)) => !["false", "0", "no"]
            .iter()
            .any(|o| s.eq_ignore_ascii_case(o)),
        Some(JsValue::Number(n)) => *n != 0.0,
        Some(v) => v.truthy(),
        None => false,
    }
}

/// `new Livefiles({ path: argv.filemanager, role, username, password, host, port })`; `safe_mode` is a port extension.
fn files_opts(args: &JsValue, on_fatal: FatalHook) -> FilemanagerOpts {
    let get = |name: &str| flag(args, name).cloned();
    FilemanagerOpts {
        path: get("filemanager"),
        role: get("role"),
        username: get("username"),
        password: get("password"),
        host: get("host"),
        port: get("port"),
        safe_mode: safe_mode_on(flag(args, "safe-mode")),
        on_fatal: Some(on_fatal),
    }
}

/// `new Holesail({ server: true, port: argv.port || 5409, host: argv.host || '127.0.0.1', secure, key })`.
fn holesail_opts(args: &JsValue) -> HolesailOpts {
    let or = |name: &str, default: JsValue| {
        let value = flag(args, name).filter(|v| v.truthy());
        Some(value.cloned().unwrap_or(default))
    };
    HolesailOpts {
        port: or("port", JsValue::Number(5409.0)),
        host: or("host", JsValue::String("127.0.0.1".into())),
        udp: None,
        ..server_opts(args)
    }
}

#[cfg(test)]
#[path = "filemanager_tests.rs"]
mod tests;
