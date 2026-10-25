// JS: libudx/src/udx.c
//! The stream entry in the socket table is removed on any end of the adapter: drop, `abort`, normal close; related — `processor_task.rs`, `handle.rs`.

use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::*;
use crate::native::async_stream::UdxAsyncStream;
use crate::native::runtime::UdxRuntime;
use crate::native::socket::UdxSocket;

async fn bound_socket(runtime: &UdxRuntime) -> (UdxSocket, SocketAddr) {
    let socket = runtime.create_socket().await.expect("create_socket");
    socket
        .bind("127.0.0.1:0".parse().expect("addr"))
        .await
        .expect("bind");
    let addr = socket.local_addr().await.expect("local_addr");
    (socket, addr)
}

fn table_len(socket: &UdxSocket) -> usize {
    let streams = socket.streams_ref();
    let guard = streams.lock().unwrap_or_else(|e| e.into_inner());
    guard.len()
}

/// Polling every 10 ms up to `limit`: after `abort()` the task's future is destroyed on the nearest tick.
async fn wait_table_len_within(socket: &UdxSocket, n: usize, limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    while table_len(socket) != n {
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    true
}

async fn wait_table_len(socket: &UdxSocket, n: usize) {
    let ok = wait_table_len_within(socket, n, Duration::from_secs(2)).await;
    assert!(ok, "table has {} entries, expected {n}", table_len(socket));
}

/// Two bound sockets with addresses.
struct Sockets {
    a: UdxSocket,
    addr_a: SocketAddr,
    b: UdxSocket,
    addr_b: SocketAddr,
}

/// Streams `ids_a[i]` on A and `ids_b[i]` on B, connected to each other; adapters A and B.
async fn pairs(
    runtime: &UdxRuntime,
    sockets: &Sockets,
    ids: &[(u32, u32)],
) -> Vec<(UdxAsyncStream, UdxAsyncStream)> {
    let Sockets {
        a: socket_a,
        addr_a,
        b: socket_b,
        addr_b,
    } = sockets;
    let mut out = Vec::new();
    for &(id_a, id_b) in ids {
        let a = runtime.create_stream(id_a).await.expect("stream a");
        let b = runtime.create_stream(id_b).await.expect("stream b");
        a.connect(socket_a, id_b, *addr_b).await.expect("connect a");
        b.connect(socket_b, id_a, *addr_a).await.expect("connect b");
        out.push((a.into_async_stream(), b.into_async_stream()));
    }
    out
}

async fn two_sockets(runtime: &UdxRuntime) -> Sockets {
    let (a, addr_a) = bound_socket(runtime).await;
    let (b, addr_b) = bound_socket(runtime).await;
    Sockets {
        a,
        addr_a,
        b,
        addr_b,
    }
}

#[tokio::test]
async fn dropped_stream_leaves_table_empty() {
    let runtime = UdxRuntime::new().expect("runtime");
    let sockets = two_sockets(&runtime).await;
    let mut set = pairs(&runtime, &sockets, &[(1, 2)]).await;
    assert_eq!(table_len(&sockets.a), 1);
    let (a, _b) = set.remove(0);
    drop(a);
    wait_table_len(&sockets.a, 0).await;
}

#[tokio::test]
async fn several_dropped_streams_leave_table_empty() {
    let runtime = UdxRuntime::new().expect("runtime");
    let sockets = two_sockets(&runtime).await;
    let set = pairs(&runtime, &sockets, &[(1, 11), (2, 12), (3, 13)]).await;
    assert_eq!(table_len(&sockets.a), 3);
    let mut keep_b = Vec::new();
    for (expected, (a, b)) in [2, 1, 0].into_iter().zip(set) {
        drop(a);
        keep_b.push(b);
        wait_table_len(&sockets.a, expected).await;
    }
}

#[tokio::test]
async fn aborted_stream_leaves_table_empty() {
    let runtime = UdxRuntime::new().expect("runtime");
    let sockets = two_sockets(&runtime).await;
    let mut set = pairs(&runtime, &sockets, &[(1, 2)]).await;
    assert_eq!(table_len(&sockets.a), 1);
    let (a, _b) = set.remove(0);
    a.abort();
    wait_table_len(&sockets.a, 0).await;
}

#[tokio::test]
async fn dropped_stream_with_unsent_data_keeps_entry_until_processor_exits_then_empty() {
    let runtime = UdxRuntime::new().expect("runtime");
    let sockets = two_sockets(&runtime).await;
    let mut set = pairs(&runtime, &sockets, &[(1, 2)]).await;
    let (mut a, _b) = set.remove(0);
    // a write without waiting for ACK: unsent data stays in the queue
    a.write_all(&vec![7u8; 512 * 1024]).await.expect("write");
    assert_eq!(table_len(&sockets.a), 1);
    drop(a);
    // without await: the processor task is still alive, so the entry is held
    assert_eq!(table_len(&sockets.a), 1);
    let ok = wait_table_len_within(&sockets.a, 0, Duration::from_secs(5)).await;
    assert!(ok, "entry not removed after the processor exited");
}

#[tokio::test]
async fn finished_stream_leaves_table_empty() {
    let runtime = UdxRuntime::new().expect("runtime");
    let sockets = two_sockets(&runtime).await;
    let mut set = pairs(&runtime, &sockets, &[(1, 2)]).await;
    let (mut a, mut b) = set.remove(0);
    a.shutdown().await.expect("shutdown a");
    b.shutdown().await.expect("shutdown b");
    let mut rest = Vec::new();
    a.read_to_end(&mut rest).await.expect("read a");
    b.read_to_end(&mut rest).await.expect("read b");
    assert_eq!(table_len(&sockets.a), 1);
    drop(a);
    wait_table_len(&sockets.a, 0).await;
}

/// Ids on one socket repeat (relay and swarm have separate counters): the guard of the old task
/// must not remove the entry of the successor that took the same id before the future was destroyed.
#[tokio::test]
async fn dropped_stream_does_not_remove_its_successors_entry() {
    let runtime = UdxRuntime::new().expect("runtime");
    let sockets = two_sockets(&runtime).await;
    let mut set = pairs(&runtime, &sockets, &[(1, 2)]).await;
    let (a, _b) = set.remove(0);
    drop(a);
    // create_stream does not yield to the scheduler: the old task is not destroyed yet
    let successor = runtime.create_stream(1).await.expect("successor");
    successor.register(&sockets.a).expect("register");
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(table_len(&sockets.a), 1, "successor entry removed");
}

#[tokio::test]
async fn registered_unconnected_stream_leaves_table_empty() {
    let runtime = UdxRuntime::new().expect("runtime");
    let sockets = two_sockets(&runtime).await;
    let stream = runtime.create_stream(1).await.expect("stream");
    stream.register(&sockets.a).expect("register");
    assert_eq!(table_len(&sockets.a), 1);
    drop(stream.into_async_stream());
    wait_table_len(&sockets.a, 0).await;
}

#[tokio::test]
async fn firewall_hook_stream_leaves_table_empty() {
    let runtime = UdxRuntime::new().expect("runtime");
    let sockets = two_sockets(&runtime).await;
    let stream = runtime.create_stream(1).await.expect("stream");
    stream
        .set_firewall_hook(&sockets.a, 2, |_, _, _| true)
        .expect("set_firewall_hook");
    assert_eq!(table_len(&sockets.a), 1);
    drop(stream.into_async_stream());
    wait_table_len(&sockets.a, 0).await;
}
