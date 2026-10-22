//! Relay options shared by the probes: command-line flags into `ConnectOpts` / `ServerOpts`, and the `PATH` line.
//! `PATH relayed|direct remote=<addr> local=<addr>` is printed once the connection is up and again on every path change.
// JS: hyperdht/lib/connect.js:53 (relayThrough), :98-106 (holepunch), :106 (relayKeepAlive)

use std::time::Duration;

use peeroxide_dht::hyperdht::{
    ConnectOpts, HolepunchFilter, PeerConnection, RelayPath, RelayThrough,
};
use peeroxide_dht::server::ServerOpts;

/// Options of the relay set that take no value.
pub const RELAY_FLAGS: [&str; 1] = ["no-holepunch"];
/// Options of the relay set that take a value.
pub const RELAY_VALUES: [&str; 4] = [
    "relay-through",
    "punch-delay-ms",
    "relay-keepalive-ms",
    "echo-pause-ms",
];

/// Parsed command line as the relay options see it.
pub trait RelayArgs {
    fn value(&self, name: &str) -> Option<&str>;
    fn has(&self, name: &str) -> bool;
}

struct Parsed {
    relay: Option<RelayThrough>,
    holepunch: Option<HolepunchFilter>,
    keep_alive: Option<Duration>,
    punch_delay: Option<Duration>,
}

/// `--echo-pause-ms`: the client waits this long once half of the payload is written (the upgrade happens meanwhile).
pub fn pause(args: &dyn RelayArgs) -> Result<Option<Duration>, String> {
    millis(args, "echo-pause-ms")
}

fn millis(args: &dyn RelayArgs, name: &str) -> Result<Option<Duration>, String> {
    let Some(text) = args.value(name) else {
        return Ok(None);
    };
    let ms: u64 = text.parse().map_err(|e| format!("--{name}: {e}"))?;
    Ok(Some(Duration::from_millis(ms)))
}

fn parse(args: &dyn RelayArgs) -> Result<Parsed, String> {
    let relay = match args.value("relay-through") {
        Some(text) => Some(RelayThrough::Key(parse_key(text)?)),
        None => None,
    };
    Ok(Parsed {
        relay,
        holepunch: args
            .has("no-holepunch")
            .then(|| HolepunchFilter::new(|_| false)),
        keep_alive: millis(args, "relay-keepalive-ms")?,
        punch_delay: millis(args, "punch-delay-ms")?,
    })
}

fn parse_key(text: &str) -> Result<[u8; 32], String> {
    let bytes = hex::decode(text).map_err(|e| format!("--relay-through: {e}"))?;
    <[u8; 32]>::try_from(bytes.as_slice()).map_err(|_| "--relay-through: need 32 bytes".into())
}

/// `ConnectOpts` of a client; the LAN shortcut stays off so the relay is not bypassed on loopback.
pub fn connect_opts(args: &dyn RelayArgs) -> Result<ConnectOpts, String> {
    let p = parse(args)?;
    let mut opts = ConnectOpts::default();
    opts.local_connection = p.relay.is_none();
    opts.relay_through = p.relay;
    opts.holepunch = p.holepunch;
    opts.punch_delay = p.punch_delay;
    if let Some(keep_alive) = p.keep_alive {
        opts.relay_keep_alive = keep_alive;
    }
    Ok(opts)
}

/// Relay fields of a server's `ServerOpts`.
pub fn apply_server_opts(args: &dyn RelayArgs, opts: &mut ServerOpts) -> Result<(), String> {
    let p = parse(args)?;
    opts.relay_through = p.relay;
    opts.holepunch = p.holepunch;
    if let Some(keep_alive) = p.keep_alive {
        opts.relay_keep_alive = keep_alive;
    }
    Ok(())
}

fn path_line(
    path: RelayPath,
    remote: Option<std::net::SocketAddr>,
    local: Option<std::net::SocketAddr>,
) -> String {
    let word = match path {
        RelayPath::Relayed => "relayed",
        _ => "direct",
    };
    let show = |a: Option<std::net::SocketAddr>| a.map_or("none".to_string(), |a| a.to_string());
    format!("PATH {word} remote={} local={}", show(remote), show(local))
}

/// Prints `PATH` now and on every later path change (a background task, ends with the connection).
pub async fn report_path(conn: &PeerConnection) {
    let peer = conn.stream.get_ref().peer();
    let remote = {
        let peer = peer.clone();
        move || peer.remote().map(|(addr, _)| addr)
    };
    let local = move || peer.local();
    let Some(mut rx) = conn.relay_path() else {
        println!("{}", path_line(RelayPath::Direct, remote(), local()));
        return;
    };
    println!("{}", path_line(*rx.borrow(), remote(), local()));
    tokio::spawn(async move {
        while rx.changed().await.is_ok() {
            println!("{}", path_line(*rx.borrow(), remote(), local()));
        }
    });
}
