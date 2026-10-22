//! Tests for `hello_addresses4`: the addresses the client announces in its first Noise message.
// JS: hyperdht/lib/connect.js:420-437 (`c.lan`)

use super::*;

mod hello_addresses4 {
    use super::*;

    #[test]
    fn without_local_connection_only_the_reflexive() {
        // given: a known reflexive address and a local interface address
        let (reflexive, local) = (peer("1.2.3.4", 5), peer("192.168.1.5", 5));
        // when: the client does not allow the LAN shortcut
        let got = hello_addresses4(Some(reflexive.clone()), vec![local], false);
        // then: the local address is not announced
        assert_eq!(got, vec![reflexive]);
    }

    #[test]
    fn without_local_connection_and_reflexive_is_empty() {
        // given: no reflexive address yet and a local interface address
        let local = vec![peer("192.168.1.5", 5)];
        // when: the client does not allow the LAN shortcut
        let got = hello_addresses4(None, local, false);
        // then: nothing is announced
        assert!(got.is_empty(), "announced {got:?}");
    }

    #[test]
    fn with_local_connection_reflexive_first() {
        // given: a reflexive address and two local interface addresses
        let reflexive = peer("1.2.3.4", 5);
        let (l1, l2) = (peer("192.168.1.5", 5), peer("10.0.0.7", 5));
        // when: the client allows the LAN shortcut
        let got = hello_addresses4(Some(reflexive.clone()), vec![l1.clone(), l2.clone()], true);
        // then: the reflexive address goes first, the local ones follow in order
        assert_eq!(got, vec![reflexive, l1, l2]);
    }
}

fn peer(host: &str, port: u16) -> Ipv4Peer {
    Ipv4Peer {
        host: host.into(),
        port,
    }
}
