// Tests of the address conversions between holesail and the fork.
use super::*;

fn peer(host: &str, port: u16) -> Ipv4Peer {
    Ipv4Peer {
        host: host.to_string(),
        port,
    }
}

#[test]
fn to_peers_keeps_order() {
    let nodes = ["1.2.3.4:5".parse().unwrap(), "6.7.8.9:10".parse().unwrap()];
    let got = to_peers(&nodes);
    assert_eq!(got.len(), 2);
    assert_eq!((got[0].host.as_str(), got[0].port), ("1.2.3.4", 5));
    assert_eq!((got[1].host.as_str(), got[1].port), ("6.7.8.9", 10));
}

#[test]
fn to_addrs_drops_non_ipv4() {
    let peers = vec![
        peer("1.2.3.4", 5),
        peer("example.com", 6),
        peer("::1", 7),
        peer("6.7.8.9", 10),
    ];
    let want: Vec<SocketAddrV4> = vec!["1.2.3.4:5".parse().unwrap(), "6.7.8.9:10".parse().unwrap()];
    assert_eq!(to_addrs(peers), want);
}
