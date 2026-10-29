#![deny(clippy::all)]

//! Blind-relay probe for `interop/probe-relay.mjs`: runs `run_relay_server` on 127.0.0.1.
//! `probe_relay relay --bootstrap h:p[,h:p] --seed <hex>` prints `LISTENING <pk>`, then
//! `STATS k=v ...` whenever the relay counters change (the API has no per-event hook); logs go to stderr.

use std::process::ExitCode;
use std::time::Duration;

use libudx::UdxRuntime;
use peeroxide_dht::blind_relay::RelayStatsSnapshot;
use peeroxide_dht::hyperdht::{self, HyperDhtConfig, KeyPair};
use peeroxide_dht::relay_service::{RelayServiceConfig, run_relay_server};
use peeroxide_dht::rpc::DhtConfig;

fn arg(argv: &[String], name: &str) -> Result<String, String> {
    let flag = format!("--{name}");
    let at = argv
        .iter()
        .position(|a| *a == flag)
        .ok_or(format!("{flag} is required"))?;
    argv.get(at + 1)
        .cloned()
        .ok_or(format!("{flag} needs a value"))
}

fn stats_line(s: &RelayStatsSnapshot) -> String {
    format!(
        "STATS sessions_accepted={} sessions_active={} pairings_requested={} pairings_matched={} \
         pairings_cancelled={} pairings_pending={} pairings_active={} streams_opened={} \
         streams_closed={} streams_errors={}",
        s.sessions_accepted,
        s.sessions_active,
        s.pairings_requested,
        s.pairings_matched,
        s.pairings_cancelled,
        s.pairings_pending,
        s.pairings_active,
        s.streams_opened,
        s.streams_closed,
        s.streams_errors
    )
}

async fn relay(argv: &[String]) -> Result<(), String> {
    let seed = hex::decode(arg(argv, "seed")?).map_err(|e| format!("--seed: {e}"))?;
    let seed = <[u8; 32]>::try_from(seed.as_slice()).map_err(|_| "--seed: need 32 bytes")?;
    let key_pair = KeyPair::from_seed(seed);
    let rt = UdxRuntime::new().map_err(|e| format!("udx runtime: {e}"))?;
    let mut dht = DhtConfig::default();
    dht.bootstrap = arg(argv, "bootstrap")?
        .split(',')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    dht.host = "0.0.0.0".to_string();
    dht.ephemeral = Some(false);
    dht.firewalled = false;
    let mut config = HyperDhtConfig::default();
    config.dht = dht;
    let (task, handle, events) = hyperdht::spawn(&rt, config)
        .await
        .map_err(|e| format!("spawn: {e}"))?;
    handle
        .bootstrapped()
        .await
        .map_err(|e| format!("bootstrap: {e}"))?;
    let (server, service) = run_relay_server(
        rt.handle(),
        handle.clone(),
        key_pair.clone(),
        events,
        RelayServiceConfig::default(),
    );
    println!("LISTENING {}", hex::encode(key_pair.public_key));
    let mut last = String::new();
    let mut tick = tokio::time::interval(Duration::from_millis(200));
    loop {
        tick.tick().await;
        if service.is_finished() {
            task.abort();
            return Err("relay task ended".to_string());
        }
        let line = stats_line(&server.stats());
        if line != last {
            println!("{line}");
            last = line;
        }
    }
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
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let result = match argv.first().map(String::as_str) {
        Some("relay") => relay(&argv).await,
        _ => Err("usage: probe_relay relay --bootstrap h:p --seed <hex>".to_string()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            println!("ERROR {e}");
            ExitCode::from(1)
        }
    }
}
