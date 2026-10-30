//! A relay whose node sits behind a consistent NAT: a client must go through a punch round with it, and
//! the pair it brokers must carry data both ways. An open relay is dialled directly, without a punch.
// JS: hyperdht/lib/server.js

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::hyperdht::{ConnectOpts, HolepunchFilter, HyperDhtError, PeerConnection, RelayPaired};
use crate::hyperdht_messages::FIREWALL_CONSISTENT;
use crate::relay_service::RelayServiceConfig;
use crate::relay_service::test_net::{NetShape, RelayNet, Who, echo_over, pair, relay_net_shaped};

const A_ID: u32 = 0x1001;
const B_ID: u32 = 0x2001;

mod relay_behind_a_nat {
    use super::*;

    mod peers_behind_a_nat_too {
        use super::*;

        #[tokio::test(flavor = "multi_thread")]
        async fn client_reaches_a_punch_round_with_the_relay() {
            // given: a relay behind a NAT and two peers behind NATs
            let n = setup(NetShape {
                relay_firewalled: true,
                peers_firewalled: true,
            })
            .await;

            // when: a client connects to the relay and we watch for its holepunch hook
            let (conn, firewall) = connect_watching_punch(&n, Duration::from_secs(15)).await;

            // then: the client entered a punch round against a consistent-NAT relay
            assert_eq!(
                firewall,
                Some(FIREWALL_CONSISTENT),
                "client never reached a punch round: relay answered without holepunch"
            );
            if let Err(e) = conn {
                panic!("punched connection failed: {e:?}");
            }
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn pair_carries_data_both_ways() {
            // given: the same shape
            let n = setup(NetShape {
                relay_firewalled: true,
                peers_firewalled: true,
            })
            .await;

            // when: the pair is formed and each side sends bytes
            let (a, b) = pair_both(&n).await;
            let (a, b) = (a.expect("A pairs"), b.expect("B pairs"));

            // then: the bytes arrive in both directions
            echo_over(&n, &a, A_ID, &b, B_ID).await;
        }
    }

    mod open_peers {
        use super::*;

        #[tokio::test(flavor = "multi_thread")]
        async fn relay_dials_open_peers_and_pairs_them() {
            // given: a relay behind a NAT and two open peers
            let n = setup(NetShape {
                relay_firewalled: true,
                peers_firewalled: false,
            })
            .await;

            // when: A and B pair on one token
            let (a, b) = pair_both(&n).await;

            // then: both pair and the relay counts the match
            assert!(a.is_ok(), "A pairs: {:?}", a.err());
            assert!(b.is_ok(), "B pairs: {:?}", b.err());
            assert!(n.relay.stats().pairings_matched >= 1);
        }
    }
}

mod open_relay {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn open_relay_is_dialled_without_a_punch() {
        // given: an open relay and peers behind NATs
        let n = setup(NetShape {
            relay_firewalled: false,
            peers_firewalled: true,
        })
        .await;

        // when: a client connects to the relay
        let (conn, firewall) = connect_watching_punch(&n, Duration::from_secs(3)).await;

        // then: it dials directly and the holepunch hook never fires
        if let Err(e) = conn {
            panic!("direct connection failed: {e:?}");
        }
        assert_eq!(firewall, None, "an open relay must not need a punch round");
    }
}

async fn setup(shape: NetShape) -> RelayNet {
    relay_net_shaped(RelayServiceConfig::default(), shape).await
}

/// A and B pair on one fresh token through the relay.
async fn pair_both(
    n: &RelayNet,
) -> (
    Result<RelayPaired, HyperDhtError>,
    Result<RelayPaired, HyperDhtError>,
) {
    let token = rand::random::<[u8; 32]>();
    tokio::join!(
        pair(n, Who::A, token, true, A_ID),
        pair(n, Who::B, token, false, B_ID)
    )
}

/// Connects A to the relay until the client's holepunch hook fires (or `deadline` passes). Returns that
/// attempt's result and the firewall the hook saw. The first handshake can race the relay's announce and
/// carry no relays, which makes the client dial directly; hence the retries.
async fn connect_watching_punch(
    n: &RelayNet,
    deadline: Duration,
) -> (Result<PeerConnection, HyperDhtError>, Option<u64>) {
    let start = Instant::now();
    loop {
        let seen = Arc::new(Mutex::new(None));
        let hook = Arc::clone(&seen);
        let opts = ConnectOpts {
            holepunch: Some(HolepunchFilter::new(move |input| {
                *hook.lock().unwrap() = Some(input.remote_firewall);
                true
            })),
            ..ConnectOpts::default()
        };
        let attempt = tokio::time::timeout(
            Duration::from_secs(10),
            n.a.handle
                .connect_with_options(&n.a.key_pair, n.relay_pk, &[], &n.a.rt, opts),
        )
        .await
        .unwrap_or_else(|_| Err(HyperDhtError::HandshakeFailed("attempt timed out".into())));
        let firewall = *seen.lock().unwrap();
        if firewall.is_some() || start.elapsed() >= deadline {
            return (attempt, firewall);
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}
