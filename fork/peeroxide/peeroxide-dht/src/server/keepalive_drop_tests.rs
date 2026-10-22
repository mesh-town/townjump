//! Re-announce withdraws the record from relays that dropped out of the new list.

use std::time::Duration;

use tokio::sync::mpsc;

use super::{Keepalive, dropped};
use crate::crypto::hash;
use crate::hyperdht::HyperDhtHandle;
use crate::hyperdht_messages::RelayInfo;
use crate::messages::Ipv4Peer;
use crate::server::actor::Command;
use crate::server::test_net::{cluster, key, node, within};

/// Number of nodes that hold a record under `target`.
async fn holders(handle: &HyperDhtHandle, target: [u8; 32]) -> usize {
    handle.lookup(target).await.expect("lookup").len()
}

/// Answers every `Relays` with `relays`, ignores the rest.
async fn play_actor(mut rx: mpsc::UnboundedReceiver<Command>, relays: Vec<RelayInfo>) {
    while let Some(command) = rx.recv().await {
        if let Command::Relays(reply) = command {
            let _ = reply.send(Some(relays.clone()));
        }
    }
}

mod reannounce {
    use super::*;

    /// The actor holds every node as a relay; the new announce picks 3: the others lose the record.
    #[tokio::test]
    async fn reannounce_withdraws_the_record_from_dropped_relays() {
        within(async {
            // given
            let cl = cluster(8).await;
            let pinger = node(&cl, false).await;
            let kp = key(1);
            let target = hash(&kp.public_key);
            pinger
                .handle
                .announce(target, &kp, &[])
                .await
                .expect("announce");
            let before = holders(&pinger.handle, target).await;
            assert!(
                before > 3,
                "only {before} holders, the test needs more than 3"
            );
            let all: Vec<_> = cl
                .addrs
                .iter()
                .map(|a| RelayInfo {
                    relay_address: a.clone(),
                    peer_address: a.clone(),
                })
                .collect();
            let (tx, rx) = mpsc::unbounded_channel();
            tokio::spawn(play_actor(rx, all));
            let keepalive = Keepalive::new(pinger.handle.clone(), tx, kp.clone());
            // when
            keepalive.reannounce().await;
            // then
            let mut now = before;
            for _ in 0..50 {
                now = holders(&pinger.handle, target).await;
                if now <= 3 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            assert!(
                now <= 3,
                "holders after re-announce: {now} (before: {before})"
            );
        })
        .await
    }
}

fn at(port: u16) -> Ipv4Peer {
    Ipv4Peer {
        host: "127.0.0.1".into(),
        port,
    }
}

fn info(ports: &[u16]) -> Vec<RelayInfo> {
    ports
        .iter()
        .map(|p| RelayInfo {
            relay_address: at(*p),
            peer_address: at(*p),
        })
        .collect()
}

mod dropped_relays {
    use super::*;

    #[test]
    fn keeps_only_relays_missing_from_the_new_list() {
        assert_eq!(
            dropped(&info(&[1, 2, 3]), &info(&[2, 4])),
            vec![at(1), at(3)]
        );
    }

    #[test]
    fn nothing_when_the_lists_match() {
        assert!(dropped(&info(&[1, 2, 3]), &info(&[3, 1, 2])).is_empty());
    }

    #[test]
    fn everything_when_the_new_list_is_empty() {
        assert_eq!(dropped(&info(&[1, 2]), &[]), vec![at(1), at(2)]);
    }

    #[test]
    fn compares_host_and_port() {
        assert_eq!(dropped(&info(&[1]), &info(&[2])), vec![at(1)]);
    }
}
