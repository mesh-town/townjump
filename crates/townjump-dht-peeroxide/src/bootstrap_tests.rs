// Tests of `bootstrap`: address parsing and mapping loopback ones to `127.0.0.1`, no network.
use super::*;

fn plan(list: &[&str]) -> Result<BootstrapPlan, DhtError> {
    let owned: Vec<String> = list.iter().map(|s| s.to_string()).collect();
    BootstrapPlan::parse(Some(&owned))
}

#[test]
fn none_is_the_public_bootstrap() {
    let public = BootstrapPlan::parse(None).expect("parse");
    assert_eq!(public.nodes.len(), DEFAULT_BOOTSTRAP.len());
    assert!(!public.loopback_only);
}

#[test]
fn loopback_spellings_become_127_0_0_1() {
    for host in [
        "localhost",
        "LocalHost",
        "0.0.0.0",
        "127.0.0.1",
        "127.1.2.3",
        "[::1]",
        "::1",
    ] {
        let got = plan(&[&format!("{host}:4000")]).expect("parse");
        assert_eq!(got.nodes, ["127.0.0.1:4000"], "host {host}");
        assert!(got.loopback_only, "host {host}");
    }
}

#[test]
fn foreign_addresses_are_kept_verbatim() {
    let got = plan(&["10.1.2.3:49737", "example.org:80"]).expect("parse");
    assert_eq!(got.nodes, ["10.1.2.3:49737", "example.org:80"]);
    assert!(!got.loopback_only);
}

#[test]
fn one_foreign_address_keeps_the_host_open() {
    let got = plan(&["127.0.0.1:1", "10.0.0.1:2"]).expect("parse");
    assert_eq!(got.nodes, ["127.0.0.1:1", "10.0.0.1:2"]);
    assert!(!got.loopback_only);
}

#[test]
fn empty_list_is_not_loopback() {
    let got = plan(&[]).expect("parse");
    assert!(got.nodes.is_empty());
    assert!(!got.loopback_only);
}

#[test]
fn garbage_is_refused_with_the_address_in_the_text() {
    for bad in [
        "nonsense",
        "127.0.0.1:port",
        "127.0.0.1:70000",
        ":80",
        "host:",
    ] {
        let error = plan(&[bad]).expect_err(bad);
        assert!(
            matches!(&error, DhtError::Backend(t) if t.contains(bad)),
            "{bad}: {error:?}"
        );
    }
}

#[test]
fn loopback_config_pins_the_host_and_leaves_the_rest() {
    let config = plan(&["localhost:9"]).expect("parse").config();
    assert_eq!(config.dht.host, LOOPBACK);
    assert_eq!(config.dht.bootstrap, ["127.0.0.1:9"]);
    assert!(!config.dht.firewalled);
    assert_eq!(
        config.dht.ephemeral,
        Some(true),
        "a persistent node clogs the testnet tables"
    );
    let open = plan(&["10.0.0.1:9"]).expect("parse").config();
    assert_eq!(open.dht.host, DhtConfig::default().host);
    assert!(open.dht.firewalled);
}
