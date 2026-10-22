// JS: holesail/src/bin/holesail.mjs (`--live` and `--connect` branches): `new Holesail`, `ready`, banner, waiting for a failure.
// See also: args.rs builds the options; facade.rs — the class itself; exit on `Fatal` (code 1); with `--save-nodes` also on a signal (130).
use super::args::{client_opts, log_level, server_opts};
use super::filemanager::shutdown_on_signal;
use super::interrupt::{SIGNAL_EXIT, signalled};
use super::nodes::{NodesOpts, start};
use super::{CliIo, emit, fail};
use crate::cli::banner::banner;
use crate::facade::{Holesail, HolesailOpts};
use townjump_dht::DhtFactory;
use townjump_jsval::JsValue;
use townjump_logger::{Fatal, Logger};

pub(super) async fn server<F: DhtFactory>(
    args: &JsValue,
    io: CliIo,
    factory: F,
    nodes: Option<NodesOpts>,
) -> i32 {
    serve(args, server_opts(args), io, factory, nodes).await
}

pub(super) async fn client<F: DhtFactory>(
    args: &JsValue,
    io: CliIo,
    factory: F,
    nodes: Option<NodesOpts>,
) -> i32 {
    serve(args, client_opts(args), io, factory, nodes).await
}

/// Creates `Holesail`, waits for `ready`, prints the banner and keeps the node until `Fatal` arrives.
async fn serve<F: DhtFactory>(
    args: &JsValue,
    opts: HolesailOpts,
    mut io: CliIo,
    factory: F,
    nodes: Option<NodesOpts>,
) -> i32 {
    let logger = Logger::new(log_level(args), io.stdout.clone(), io.stderr.clone());
    let (fatal, mut fatals) = Fatal::channel();
    let holesail = Holesail::new(factory, opts, logger, fatal, &mut io.random);
    let mut holesail = match holesail {
        Ok(holesail) => holesail,
        Err(error) => return fail(&io, &error.to_string()),
    };
    if let Err(error) = holesail.ready().await {
        return fail(&io, &error.to_string());
    }
    emit(&io.stdout, &banner(&holesail.info()));
    let saver = nodes.and_then(|o| holesail.dht().map(|dht| start(dht, &o)));
    let Some(saver) = saver else {
        let message = fatals.recv().await.unwrap_or_default();
        return fail(&io, &message);
    };
    let mut signals = (io.interrupt)();
    tokio::select! {
        biased;
        message = fatals.recv() => {
            saver.finish().await;
            emit(&io.stderr, &format!("{}\n", message.unwrap_or_default()));
            1
        }
        () = signalled(&mut signals) => {
            shutdown_on_signal(&mut signals, saver.finish()).await;
            SIGNAL_EXIT
        }
    }
}
