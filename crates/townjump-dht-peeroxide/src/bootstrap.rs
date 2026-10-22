// JS: dht-rpc (`bootstrap` parsing), `TOWNJUMP_BOOTSTRAP`: loopback addresses → `127.0.0.1`).
// See also: factory.rs builds the fork config from the plan; a malformed address is rejected here, the fork silently skips it.
use peeroxide_dht::hyperdht::{DEFAULT_BOOTSTRAP, HyperDhtConfig};
use peeroxide_dht::rpc::DhtConfig;
use std::net::IpAddr;
use townjump_dht::DhtError;

const LOOPBACK: &str = "127.0.0.1";

/// Node bootstrap after parsing: addresses for the fork and a flag "the whole network is on loopback" (testnet).
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct BootstrapPlan {
    pub nodes: Vec<String>,
    pub loopback_only: bool,
}

struct Address {
    host: String,
    port: u16,
    loopback: bool,
}

impl BootstrapPlan {
    /// `None` — the public `DEFAULT_BOOTSTRAP` nodes; otherwise every address is `host:port`, a malformed one is `Backend`.
    pub fn parse(bootstrap: Option<&[String]>) -> Result<BootstrapPlan, DhtError> {
        let Some(list) = bootstrap else {
            return Ok(BootstrapPlan::public());
        };
        let parsed: Vec<Address> = list
            .iter()
            .map(|s| parse_address(s))
            .collect::<Result<_, _>>()?;
        let loopback_only = !parsed.is_empty() && parsed.iter().all(|a| a.loopback);
        let nodes = parsed.into_iter().map(Address::render).collect();
        Ok(BootstrapPlan {
            nodes,
            loopback_only,
        })
    }

    fn public() -> BootstrapPlan {
        let nodes = DEFAULT_BOOTSTRAP.iter().map(|s| s.to_string()).collect();
        BootstrapPlan {
            nodes,
            loopback_only: false,
        }
    }

    /// Node config; on loopback — host `127.0.0.1`, not behind NAT, ephemeral, otherwise the fork defaults. Ephemeral, like a JS node for its first ~20 minutes
    /// (dht-rpc `STABLE_TICKS`): a persistent process killed when the run ends stays a dead entry in the testnet
    /// node tables, and every next request waits for its timeout.
    pub fn config(&self) -> HyperDhtConfig {
        let mut dht = DhtConfig::default();
        dht.bootstrap = self.nodes.clone();
        if self.loopback_only {
            dht.host = LOOPBACK.to_string();
            dht.firewalled = false;
            dht.ephemeral = Some(true);
        }
        let mut config = HyperDhtConfig::default();
        config.dht = dht;
        config
    }
}

impl Address {
    fn render(self) -> String {
        let host = if self.loopback { LOOPBACK } else { &self.host };
        format!("{host}:{}", self.port)
    }
}

fn parse_address(text: &str) -> Result<Address, DhtError> {
    let invalid = || DhtError::Backend(format!("Invalid bootstrap address: {text:?}"));
    let (host, port) = text.rsplit_once(':').ok_or_else(invalid)?;
    let port: u16 = port.parse().map_err(|_| invalid())?;
    if host.is_empty() {
        return Err(invalid());
    }
    Ok(Address {
        host: host.to_string(),
        port,
        loopback: is_loopback(host),
    })
}

/// `localhost`, `127.*`, `0.0.0.0`, `::1`, `::` (with or without square brackets).
fn is_loopback(host: &str) -> bool {
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    let named = bare.eq_ignore_ascii_case("localhost");
    named
        || bare
            .parse::<IpAddr>()
            .is_ok_and(|ip| ip.is_loopback() || ip.is_unspecified())
}

#[cfg(test)]
#[path = "bootstrap_tests.rs"]
mod tests;
