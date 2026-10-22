//! The relay addresses of a connect are kept per target key (JS `_relayAddressesCache`) and
//! lead the next connect to it as `pre`, next to FIND_PEER. Seen in the DEBUG log of the connect;
//! 127.0.0.1 only.
mod common;

use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{cluster, serve_key, spawn_node, within};
use peeroxide_dht::hyperdht::KeyPair;
use peeroxide_dht::messages::Ipv4Peer;

#[derive(Clone, Default)]
struct Sink(Arc<Mutex<Vec<u8>>>);

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("sink").extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Sink {
    fn take(&self) -> String {
        let bytes = std::mem::take(&mut *self.0.lock().expect("sink"));
        String::from_utf8(bytes).expect("utf-8 log")
    }
}

#[tokio::test]
async fn second_connect_reads_the_cache_and_still_connects_fast() {
    within(async {
        let sink = Sink::default();
        let writer = sink.clone();
        let _log = tracing::subscriber::set_default(
            tracing_subscriber::fmt()
                .with_max_level(tracing::Level::DEBUG)
                .with_ansi(false)
                .with_writer(move || writer.clone())
                .finish(),
        );
        let (_nodes, bootstrap) = cluster(3).await;
        let key = KeyPair::from_seed([0x77; 32]);
        let _served = serve_key(&bootstrap, &key, false).await;
        let client = spawn_node(bootstrap, true).await;
        let me = KeyPair::from_seed([0x78; 32]);
        let black_hole = std::net::UdpSocket::bind("127.0.0.1:0").expect("bind");
        let dead = Ipv4Peer {
            host: "127.0.0.1".to_string(),
            port: black_hole.local_addr().expect("addr").port(),
        };

        // First connect: nothing cached; the dead relay is explicit (the server may answer with
        // its own relay addresses, then those are stored instead of the tried ones).
        let first = client
            .handle
            .connect_with_nodes(&me, key.public_key, &[dead], &client.rt)
            .await;
        assert!(first.is_ok(), "{:?}", first.err());
        let log = sink.take();
        assert!(log.contains("relay cache set n="), "{log}");
        assert!(
            log.contains("src=payload") || log.contains("src=tried"),
            "{log}"
        );

        // Second connect: the cache supplies `pre`, FIND_PEER runs beside it.
        let started = Instant::now();
        let second = client.handle.connect(&me, key.public_key, &client.rt).await;
        let elapsed = started.elapsed();
        assert!(second.is_ok(), "{:?}", second.err());
        assert!(elapsed < Duration::from_secs(3), "{elapsed:?}");
        let log = sink.take();
        assert!(log.contains("connect: pre-relays n="), "{log}");
        assert!(log.contains("(cache hit)"), "{log}");
    })
    .await;
}

#[tokio::test]
async fn first_connect_is_a_cache_miss() {
    within(async {
        let sink = Sink::default();
        let writer = sink.clone();
        let _log = tracing::subscriber::set_default(
            tracing_subscriber::fmt()
                .with_max_level(tracing::Level::DEBUG)
                .with_ansi(false)
                .with_writer(move || writer.clone())
                .finish(),
        );
        let (_nodes, bootstrap) = cluster(3).await;
        let key = KeyPair::from_seed([0x79; 32]);
        let _served = serve_key(&bootstrap, &key, false).await;
        let client = spawn_node(bootstrap, true).await;
        let me = KeyPair::from_seed([0x7a; 32]);

        let first = client.handle.connect(&me, key.public_key, &client.rt).await;
        assert!(first.is_ok(), "{:?}", first.err());
        let log = sink.take();
        assert!(
            log.contains("connect: pre-relays n=0 (cache miss)"),
            "{log}"
        );
    })
    .await;
}
