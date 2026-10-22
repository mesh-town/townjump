use super::*;

#[test]
fn config_defaults_enable_adaptive() {
    let c = DhtConfig::default();
    assert!(c.adaptive);
    assert_eq!(c.stable_ticks, 240);
    assert_eq!(c.ephemeral, None);
}

#[test]
fn mode_default_is_ephemeral_and_firewalled() {
    let m = DhtMode::default();
    assert!(m.ephemeral && m.firewalled && !m.adaptive);
    assert_eq!(m.address, None);
}
