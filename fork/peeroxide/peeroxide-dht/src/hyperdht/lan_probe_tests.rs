//! Tests for [`super`]: the one-byte probe against live loopback sockets (real time, generous limits).

use std::net::{SocketAddr, UdpSocket};
use std::time::Duration;

use libudx::UdxRuntime;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::timeout;

use super::LanProber;
use crate::holepuncher::Holepuncher;
use crate::hyperdht_messages::{FIREWALL_CONSISTENT, FIREWALL_UNKNOWN};
use crate::messages::Ipv4Peer;
use crate::socket_pool::SocketPool;

mod lan_prober {
    use super::*;

    #[tokio::test]
    async fn passive_puncher_answers_the_probe() {
        // given
        let rt = UdxRuntime::new().expect("runtime");
        let (_passive, port) = passive_puncher(&rt).await;
        let mut prober = LanProber::new(&rt, FIREWALL_CONSISTENT)
            .await
            .expect("prober");

        // when
        let got = timeout(secs(10), prober.reach(&peer("127.0.0.1", port))).await;

        // then
        let expected: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
        assert_eq!(got.expect("no answer within 10 s"), Some(expected));
    }

    #[tokio::test]
    async fn silent_socket_gets_no_answer() {
        // given: a bound socket that nobody reads
        let rt = UdxRuntime::new().expect("runtime");
        let silent = UdpSocket::bind("127.0.0.1:0").expect("bind");
        let target = silent.local_addr().expect("addr");
        let mut prober = LanProber::new(&rt, FIREWALL_CONSISTENT)
            .await
            .expect("prober");

        // when
        let got = timeout(secs(1), prober.reach(&peer("127.0.0.1", target.port()))).await;

        // then
        assert!(got.is_err(), "a silent socket produced {got:?}");
    }

    #[tokio::test]
    async fn unparsable_host_fails_at_once() {
        // given
        let rt = UdxRuntime::new().expect("runtime");
        let mut prober = LanProber::new(&rt, FIREWALL_CONSISTENT)
            .await
            .expect("prober");

        // when
        let got = timeout(secs(5), prober.reach(&peer("not-an-ip", 1))).await;

        // then
        assert_eq!(got.expect("the probe hung on an unparsable host"), None);
    }

    #[tokio::test]
    async fn socket_is_the_one_that_probed() {
        // given: a responder that learns the probe's source address and answers it with one byte
        let rt = UdxRuntime::new().expect("runtime");
        let (responder, responder_addr) = bind_loopback();
        let seen_source = reflect_one_probe(responder);
        let mut prober = LanProber::new(&rt, FIREWALL_CONSISTENT)
            .await
            .expect("prober");

        // when
        let got = timeout(
            secs(10),
            prober.reach(&peer("127.0.0.1", responder_addr.port())),
        )
        .await;

        // then
        assert_eq!(got.expect("no answer within 10 s"), Some(responder_addr));
        let source = seen_source.await.expect("responder task");
        let socket = prober.socket().expect("prober socket");
        let local = socket.local_addr().await.expect("local addr");
        assert_eq!(
            local.port(),
            source.port(),
            "the probe left from another socket than socket()"
        );
    }
}

// ---- setup and helpers ----

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn peer(host: &str, port: u16) -> Ipv4Peer {
    Ipv4Peer {
        host: host.to_string(),
        port,
    }
}

/// A passive (non-initiating) puncher, as the server runs one; gives it and its primary port.
async fn passive_puncher(rt: &UdxRuntime) -> (Holepuncher, u16) {
    let (tx, _rx) = mpsc::unbounded_channel();
    let pool = SocketPool::new("0.0.0.0".into());
    let puncher = Holepuncher::new(&pool, rt, true, false, FIREWALL_UNKNOWN, tx)
        .await
        .expect("passive puncher");
    let addr = puncher
        .primary_socket()
        .expect("primary socket")
        .socket
        .local_addr()
        .await
        .expect("local addr");
    (puncher, addr.port())
}

fn bind_loopback() -> (UdpSocket, SocketAddr) {
    let sock = UdpSocket::bind("127.0.0.1:0").expect("bind");
    let addr = sock.local_addr().expect("local addr");
    (sock, addr)
}

/// Waits for one datagram on `sock`, answers its sender with one byte and yields the sender's address.
/// A passive puncher never reports who probed it, so the test needs this to learn the source.
fn reflect_one_probe(sock: UdpSocket) -> JoinHandle<SocketAddr> {
    sock.set_read_timeout(Some(secs(20))).expect("read timeout");
    tokio::task::spawn_blocking(move || {
        let mut buf = [0u8; 16];
        let (_, source) = sock.recv_from(&mut buf).expect("probe arrived");
        sock.send_to(&[0u8], source).expect("reflect");
        source
    })
}
