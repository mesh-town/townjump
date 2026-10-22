// Tests of `node_mode`: fork `DhtMode` → `NodeMode`.
use super::*;
use peeroxide_dht::messages::Ipv4Peer;

fn mode_with(ephemeral: bool, host: Option<&str>) -> DhtMode {
    let mut mode = DhtMode::default();
    mode.ephemeral = ephemeral;
    mode.address = host.map(|h| Ipv4Peer {
        host: h.to_string(),
        port: 5,
    });
    mode
}

mod numeric_ipv4_address {
    use super::*;

    #[test]
    fn gives_persistent_with_address() {
        let got = node_mode(&mode_with(false, Some("1.2.3.4")));
        assert!(got.persistent);
        assert_eq!(got.address, Some("1.2.3.4:5".parse().unwrap()));
    }
}

mod ephemeral_mode {
    use super::*;

    #[test]
    fn is_not_persistent() {
        assert!(!node_mode(&mode_with(true, None)).persistent);
    }
}

mod adaptive_flag {
    use super::*;

    #[test]
    fn is_copied() {
        let mut mode = mode_with(true, None);
        mode.adaptive = true;
        assert!(node_mode(&mode).adaptive);
        mode.adaptive = false;
        assert!(!node_mode(&mode).adaptive);
    }
}

mod non_ipv4_host {
    use super::*;

    #[test]
    fn gives_no_address() {
        for host in ["::1", "garbage", ""] {
            let got = node_mode(&mode_with(false, Some(host)));
            assert_eq!(got.address, None, "host {host:?}");
            assert!(got.persistent);
        }
    }
}
