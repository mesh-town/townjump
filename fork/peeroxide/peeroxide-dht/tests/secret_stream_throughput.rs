#![deny(clippy::all)]

//! Tunnel speed guard: SecretStream over libudx on loopback, release ≥ ~2.7 MiB/s.
//!
//! Release only: in debug the encryption is CPU-bound and the gap between correct and
//! broken pacing is too small for the threshold. Run: `cargo test --release -p peeroxide-dht
//! --test secret_stream_throughput`. `read` is called via the trait (`AsyncReadExt::read(&mut ..)`):
//! the same-named method of `SecretStream` itself shadows it.

use std::net::SocketAddr;
use std::time::Duration;

use libudx::UdxRuntime;
use peeroxide_dht::noise::generate_keypair;
use peeroxide_dht::secret_stream::SecretStream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const TOTAL: usize = 8 * 1024 * 1024;
const CHUNK: usize = 16 * 1024;
const TIMEOUT: Duration = Duration::from_secs(3);

#[tokio::test]
#[cfg_attr(
    debug_assertions,
    ignore = "debug: encryption is CPU-bound, run with --release"
)]
async fn secret_stream_loopback_floor() {
    tokio::time::timeout(TIMEOUT, async {
        let runtime = UdxRuntime::new().expect("runtime");
        let localhost: SocketAddr = "127.0.0.1:0".parse().expect("addr");
        let socket_a = runtime.create_socket().await.expect("socket_a");
        socket_a.bind(localhost).await.expect("bind_a");
        let addr_a = socket_a.local_addr().await.expect("addr_a");
        let socket_b = runtime.create_socket().await.expect("socket_b");
        socket_b.bind(localhost).await.expect("bind_b");
        let addr_b = socket_b.local_addr().await.expect("addr_b");
        let stream_a = runtime.create_stream(1).await.expect("stream_a");
        let stream_b = runtime.create_stream(2).await.expect("stream_b");
        stream_a
            .connect(&socket_a, 2, addr_b)
            .await
            .expect("connect a");
        stream_b
            .connect(&socket_b, 1, addr_a)
            .await
            .expect("connect b");

        let raw_a = stream_a.into_async_stream();
        let raw_b = stream_b.into_async_stream();
        let (ss_a, ss_b) = tokio::join!(
            SecretStream::new(true, raw_a, generate_keypair()),
            SecretStream::new(false, raw_b, generate_keypair())
        );
        let mut ss_a = ss_a.expect("handshake a");
        let mut ss_b = ss_b.expect("handshake b");

        let payload = vec![0xA5u8; TOTAL];
        let writer = tokio::spawn(async move {
            for chunk in payload.chunks(CHUNK) {
                ss_a.write_all(chunk).await.expect("write_all");
            }
            ss_a.flush().await.expect("flush");
            ss_a
        });

        let mut received = 0usize;
        let mut buf = vec![0u8; 64 * 1024];
        while received < TOTAL {
            let n = AsyncReadExt::read(&mut ss_b, &mut buf).await.expect("read");
            assert!(n > 0, "EOF after {received} bytes");
            received += n;
        }
        let _keep = writer.await.expect("writer panicked");
    })
    .await
    .expect("secret stream throughput below the floor: timed out");
}
