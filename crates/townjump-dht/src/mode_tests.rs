// Tests of `NodeMode`: the value a backend without modes reports.
use super::*;

mod default {
    use super::*;

    #[test]
    fn is_ephemeral_firewalled_without_address() {
        let mode = NodeMode::default();

        assert!(!mode.persistent, "ephemeral by default");
        assert!(!mode.adaptive, "not adaptive by default");
        assert!(
            mode.firewalled,
            "unknown reachability is reported as behind a firewall"
        );
        assert_eq!(mode.address, None);
    }
}
