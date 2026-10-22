#![deny(clippy::all)]

//! Probe: a small CLI used by `interop/probe-dht.mjs` to check the
//! server/client paths of `peeroxide-dht` against the JS `hyperdht` on a
//! local testnet.
//!
//! ```text
//! probe connect --bootstrap h:p[,h:p] --server <pk hex> [--seed hex] [--bytes N] [--firewalled]
//! probe serve   --bootstrap h:p[,h:p] --seed hex [--private <pk hex>] [--panic-firewall <pk hex>] [--firewalled]
//! ```
//!
//! The node binds `127.0.0.1` and is not firewalled unless `--firewalled`
//! is given. Progress goes to stdout, one line per event:
//! `LISTENING <pk>`, `INCOMING <pk>`, `ECHOED <pk> <n>`, `CONNECTED <pk>`,
//! `ECHO <n> <sha256>`, `ERROR <text>` (exit code 1). Logs go to stderr
//! (`RUST_LOG`, default `warn`).
//!
//! `serve` announces the key of `--seed`, prints `INCOMING` for every accepted
//! connection and echoes it back (`ECHOED <pk> <n>` when the client half-closes
//! and the echo is finished). `--private <pk>` makes the firewall refuse every
//! client but `<pk>`; `--panic-firewall <pk>` makes it panic for `<pk>`.
//!
//! `connect` sends `--bytes` bytes (byte `i` is `i % 251`) in 64 KiB chunks,
//! half-closes the stream and reads the echo to the end; `ECHO` carries the
//! length and the sha256 of what came back.

#[path = "probe_support/relay_opts.rs"]
mod relay_opts;

use std::collections::{HashMap, HashSet};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use libudx::UdxRuntime;
use peeroxide_dht::hyperdht::{
    self, HyperDhtConfig, HyperDhtError, HyperDhtHandle, KeyPair, PeerConnection, ServerEvent,
};
use peeroxide_dht::rpc::DhtConfig;
use peeroxide_dht::server::{Firewall, FirewallInput, Server, ServerOpts};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::task::JoinHandle;

const CHUNK: usize = 64 * 1024;
const DEFAULT_BYTES: usize = 1 << 20;
const ECHO_TIMEOUT: Duration = Duration::from_secs(40);
/// Options that take no value.
const FLAGS: [&str; 2] = ["firewalled", relay_opts::RELAY_FLAGS[0]];
/// Options that take a value.
const VALUES: [&str; 6] = [
    "bootstrap",
    "server",
    "seed",
    "bytes",
    "private",
    "panic-firewall",
];

type DhtTask = JoinHandle<Result<(), HyperDhtError>>;

/// Parsed command line: the sub-command, `--key value` pairs and bare flags.
struct Args {
    cmd: String,
    values: HashMap<String, String>,
    flags: HashSet<String>,
}

impl Args {
    fn parse(mut argv: impl Iterator<Item = String>) -> Result<Self, String> {
        let cmd = argv.next().ok_or("usage: probe connect|serve [options]")?;
        let mut values = HashMap::new();
        let mut flags = HashSet::new();
        while let Some(arg) = argv.next() {
            let name = arg
                .strip_prefix("--")
                .ok_or_else(|| format!("unexpected argument {arg}"))?
                .to_string();
            if FLAGS.contains(&name.as_str()) {
                flags.insert(name);
                continue;
            }
            if !VALUES.contains(&name.as_str())
                && !relay_opts::RELAY_VALUES.contains(&name.as_str())
            {
                return Err(format!("unknown option --{name}"));
            }
            let value = argv
                .next()
                .ok_or_else(|| format!("--{name} needs a value"))?;
            values.insert(name, value);
        }
        Ok(Self { cmd, values, flags })
    }

    fn get(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(String::as_str)
    }

    fn required(&self, name: &str) -> Result<&str, String> {
        self.get(name)
            .ok_or_else(|| format!("--{name} is required"))
    }

    fn flag(&self, name: &str) -> bool {
        self.flags.contains(name)
    }
}

impl relay_opts::RelayArgs for Args {
    fn value(&self, name: &str) -> Option<&str> {
        self.get(name)
    }

    fn has(&self, name: &str) -> bool {
        self.flag(name)
    }
}

fn parse_hex32(name: &str, text: &str) -> Result<[u8; 32], String> {
    let bytes = hex::decode(text).map_err(|e| format!("--{name}: {e}"))?;
    <[u8; 32]>::try_from(bytes.as_slice()).map_err(|_| format!("--{name}: need 32 bytes"))
}

fn pattern(i: usize) -> u8 {
    (i % 251) as u8
}

/// Starts a DHT node on `127.0.0.1` bootstrapped from `--bootstrap`.
async fn spawn_dht(
    rt: &UdxRuntime,
    args: &Args,
) -> Result<(DhtTask, HyperDhtHandle, UnboundedReceiver<ServerEvent>), String> {
    let mut dht = DhtConfig::default();
    dht.bootstrap = args
        .required("bootstrap")?
        .split(',')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    dht.host = "127.0.0.1".to_string();
    dht.ephemeral = Some(false);
    dht.firewalled = args.flag("firewalled");
    let mut config = HyperDhtConfig::default();
    config.dht = dht;
    let (task, handle, events) = hyperdht::spawn(rt, config)
        .await
        .map_err(|e| format!("spawn: {e}"))?;
    handle
        .bootstrapped()
        .await
        .map_err(|e| format!("bootstrap: {e}"))?;
    Ok((task, handle, events))
}

/// Sends the pattern, half-closes, reads the echo to the end: `(length, sha256)`.
async fn echo(
    conn: PeerConnection,
    bytes: usize,
    pause: Option<Duration>,
) -> Result<(usize, String), String> {
    let PeerConnection { stream, socket, .. } = conn;
    let (mut rd, mut wr) = stream.into_split();
    let write = async {
        let mut sent = 0;
        while sent < bytes {
            let n = CHUNK.min(bytes - sent);
            let chunk: Vec<u8> = (sent..sent + n).map(pattern).collect();
            wr.write(&chunk).await.map_err(|e| format!("write: {e}"))?;
            sent += n;
            if let (Some(pause), true) = (pause, sent >= bytes / 2 && sent - n < bytes / 2) {
                tokio::time::sleep(pause).await;
            }
        }
        wr.shutdown().await.map_err(|e| format!("shutdown: {e}"))
    };
    let read = async {
        let mut hasher = Sha256::new();
        let mut total = 0;
        while let Some(chunk) = rd.read().await.map_err(|e| format!("read: {e}"))? {
            hasher.update(&chunk);
            total += chunk.len();
        }
        Ok::<_, String>((total, hex::encode(hasher.finalize())))
    };
    let (written, read) = tokio::time::timeout(ECHO_TIMEOUT, async { tokio::join!(write, read) })
        .await
        .map_err(|_| "echo: timeout".to_string())?;
    drop(socket);
    written?;
    read
}

async fn connect(args: &Args) -> Result<(), String> {
    let server = parse_hex32("server", args.required("server")?)?;
    let key_pair = match args.get("seed") {
        Some(seed) => KeyPair::from_seed(parse_hex32("seed", seed)?),
        None => KeyPair::generate(),
    };
    let bytes = match args.get("bytes") {
        Some(n) => n.parse().map_err(|e| format!("--bytes: {e}"))?,
        None => DEFAULT_BYTES,
    };
    let rt = UdxRuntime::new().map_err(|e| format!("udx runtime: {e}"))?;
    let (task, dht, _events) = spawn_dht(&rt, args).await?;
    let opts = relay_opts::connect_opts(args)?;
    let conn = dht
        .connect_with_options(&key_pair, server, &[], &rt, opts)
        .await
        .map_err(|e| format!("connect: {e}"))?;
    println!("CONNECTED {}", hex::encode(conn.remote_public_key));
    relay_opts::report_path(&conn).await;
    let (total, digest) = echo(conn, bytes, relay_opts::pause(args)?).await?;
    println!("ECHO {total} {digest}");
    dht.destroy().await.ok();
    task.abort();
    Ok(())
}

/// Firewall of the server: `--private <pk>` refuses everyone but that key,
/// `--panic-firewall <pk>` panics for that key (a panic counts as a refusal).
/// Without either option there is no callback and every client is accepted.
fn build_firewall(args: &Args) -> Result<Option<Firewall>, String> {
    let key = |name: &str| args.get(name).map(|k| parse_hex32(name, k)).transpose();
    let (private, panics) = (key("private")?, key("panic-firewall")?);
    if private.is_none() && panics.is_none() {
        return Ok(None);
    }
    let firewall: Firewall = Arc::new(move |input: &FirewallInput<'_>| {
        assert!(
            panics != Some(*input.remote_public_key),
            "probe: panic-firewall"
        );
        private.is_some_and(|allowed| allowed != *input.remote_public_key)
    });
    Ok(Some(firewall))
}

/// Echoes the stream back until the client half-closes, then half-closes
/// itself: returns the number of bytes echoed.
async fn echo_back(conn: PeerConnection) -> Result<usize, String> {
    let PeerConnection { stream, socket, .. } = conn;
    let (mut rd, mut wr) = stream.into_split();
    let mut total = 0;
    while let Some(chunk) = rd.read().await.map_err(|e| format!("read: {e}"))? {
        wr.write(&chunk).await.map_err(|e| format!("write: {e}"))?;
        total += chunk.len();
    }
    wr.shutdown().await.map_err(|e| format!("shutdown: {e}"))?;
    drop(socket);
    Ok(total)
}

/// One incoming connection: `INCOMING`, then `ECHOED` (or `ERROR`, the server keeps running).
async fn handle_incoming(conn: PeerConnection) {
    let key = hex::encode(conn.remote_public_key);
    println!("INCOMING {key}");
    relay_opts::report_path(&conn).await;
    match echo_back(conn).await {
        Ok(total) => println!("ECHOED {key} {total}"),
        Err(e) => println!("ERROR echo {key}: {e}"),
    }
}

async fn serve(args: &Args) -> Result<(), String> {
    let key_pair = KeyPair::from_seed(parse_hex32("seed", args.required("seed")?)?);
    let mut opts = ServerOpts::default();
    opts.firewall = build_firewall(args)?;
    relay_opts::apply_server_opts(args, &mut opts)?;
    let rt = UdxRuntime::new().map_err(|e| format!("udx runtime: {e}"))?;
    let (task, dht, events) = spawn_dht(&rt, args).await?;
    let (server, mut incoming) =
        Server::new(dht.clone(), events, UdxRuntime::shared(rt.handle()), opts);
    server
        .listen(key_pair.clone())
        .await
        .map_err(|e| format!("listen: {e}"))?;
    println!("LISTENING {}", hex::encode(key_pair.public_key));
    while let Some(conn) = incoming.recv().await {
        tokio::spawn(handle_incoming(conn));
    }
    server.close().await.ok();
    dht.destroy().await.ok();
    task.abort();
    Ok(())
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();
    let args = match Args::parse(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(e) => return fail(&e),
    };
    let result = match args.cmd.as_str() {
        "serve" => serve(&args).await,
        "connect" => connect(&args).await,
        other => Err(format!("unknown command {other}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => fail(&e),
    }
}

fn fail(text: &str) -> ExitCode {
    println!("ERROR {text}");
    ExitCode::from(1)
}
