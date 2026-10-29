//! The server stream follows the birthday socket that got the client's first UDX packet.
//!
//! A server behind a RANDOM NAT opens 256 sockets; the client (CONSISTENT) answers one of them and
//! sends its SYN there. File descriptors are per process, so the scenarios run in a child process
//! with a raised limit: a wrapper test re-executes this test binary with `C2_SCENARIO` set and
//! requires exit code 0. `c2_child` is a no-op unless that variable is present.

use super::*;
use crate::hyperdht::alloc_stream_id;
use crate::hyperdht_messages::{FIREWALL_CONSISTENT, FIREWALL_RANDOM};
use crate::server::actor::Internal;
use crate::server::conn::{FollowStream, adopt_moved_socket, arm_hook};
use libudx::UdxStream;
use std::net::SocketAddr;
use tokio::task::JoinHandle;

const ID: u64 = 1;
const CLIENT_ID: u32 = 4242;

fn run_child(scenario: &str) {
    let exe = std::env::current_exe().expect("current_exe");
    let out = std::process::Command::new("sh")
        .args(["-c", "ulimit -n 4096 2>/dev/null; exec \"$0\" \"$@\""])
        .arg(exe)
        .args([
            "server::punch::rebind_tests::c2_child",
            "--exact",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("C2_SCENARIO", scenario)
        .output()
        .expect("spawn child");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    println!("--- child {scenario}\n{text}");
    assert!(
        out.status.success(),
        "child {scenario} failed ({:?}):\n{text}",
        out.status
    );
}

/// The server side: a RANDOM puzzle that punches towards the verified client address.
struct Server {
    state: PunchState,
    stream: UdxStream,
    primary: UdxSocket,
    primary_port: u16,
    irx: mpsc::UnboundedReceiver<Internal>,
    _pool: SocketPool,
    _rt: UdxRuntime,
}

async fn server_for(client: SocketAddr) -> Server {
    let pool = SocketPool::new("127.0.0.1".into());
    let rt = UdxRuntime::new().expect("runtime");
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut puncher = Holepuncher::new(&pool, &rt, true, false, FIREWALL_CONSISTENT, tx)
        .await
        .expect("Holepuncher::new");
    puncher.nat.firewall = FIREWALL_RANDOM;
    let peer = Ipv4Peer {
        host: "127.0.0.1".into(),
        port: client.port(),
    };
    puncher.update_remote(true, FIREWALL_CONSISTENT, &[peer], Some("127.0.0.1"));
    let (primary, primary_port) = socket_and_port(&puncher).await.expect("socket_and_port");
    let stream = rt
        .create_stream(alloc_stream_id())
        .await
        .expect("server stream");
    let (itx, irx) = mpsc::unbounded_channel();
    arm_hook(&stream, &primary, CLIENT_ID, ID, &itx).expect("arm_hook");
    let mut state = PunchState::new(puncher, Vec::new());
    state.follow(FollowStream::new(ID, itx.clone()));
    Server {
        state,
        stream,
        primary,
        primary_port,
        irx,
        _pool: pool,
        _rt: rt,
    }
}

/// Waits for the first probe from a server socket other than the primary one; the client answers
/// it, so the server sees the client on that birthday socket. Returns its address.
async fn find_birthday(
    sock: &UdxSocket,
    rx: &mut mpsc::UnboundedReceiver<libudx::Datagram>,
    primary_port: u16,
) -> SocketAddr {
    let found = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let d = rx.recv().await.expect("client socket closed");
            if d.data.len() == 1 && d.addr.port() != primary_port {
                return d.addr;
            }
        }
    })
    .await
    .expect("no probe from a birthday socket within 10 s");
    sock.send_to(&[0], found).expect("answer the probe");
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let d = rx.recv().await.expect("client socket closed");
            if d.data.len() == 1 && d.addr == found {
                return;
            }
        }
    })
    .await
    .expect("no answer from the birthday socket within 10 s");
    found
}

/// The client side: a bare socket the server punches towards, and its stream to the birthday
/// socket `j` that answered; the stream writes `c2` and reads the server's answer.
struct Client {
    addr: SocketAddr,
    task: JoinHandle<Result<Option<Vec<u8>>, libudx::UdxError>>,
}

async fn start(
    server: &mut Server,
    client_sock: &UdxSocket,
    rt: UdxRuntime,
    j: SocketAddr,
) -> Client {
    let server_id = server.stream.local_id();
    let sock = client_sock.clone();
    let addr = sock.local_addr().await.expect("local_addr");
    let task = tokio::spawn(async move {
        let mut s = rt.create_stream(CLIENT_ID).await?;
        s.connect(&sock, server_id, j).await?;
        s.write(b"c2").await?;
        s.read().await
    });
    Client { addr, task }
}

async fn punched_server() -> (Server, UdxSocket, UdxRuntime, SocketAddr) {
    let rt = UdxRuntime::new().expect("client runtime");
    let client_sock = rt.create_socket().await.expect("client socket");
    client_sock
        .bind("127.0.0.1:0".parse().unwrap())
        .await
        .expect("bind");
    let client_addr = client_sock.local_addr().await.expect("local_addr");
    let mut crx = client_sock.recv_start().expect("recv_start");
    let mut server = server_for(client_addr).await;
    assert!(server.state.start_punch(true));
    let j = find_birthday(&client_sock, &mut crx, server.primary_port).await;
    // Keep the client's receive lane open for the whole scenario.
    std::mem::forget(crx);
    (server, client_sock, rt, j)
}

async fn follow_scenario() {
    let (mut server, client_sock, rt, j) = punched_server().await;
    let client = start(&mut server, &client_sock, rt, j).await;
    tokio::time::timeout(Duration::from_secs(5), async {
        let arrived = server.irx.recv().await;
        let Some(Internal::Arrived { id, addr }) = arrived else {
            panic!("no arrival on the birthday socket within 5 s");
        };
        assert_eq!((id, addr), (ID, client.addr));
        let moved = server.state.moved_socket();
        let socket = server.primary.clone();
        let socket = adopt_moved_socket(&server.stream, socket, moved, CLIENT_ID, addr).await;
        assert_eq!(socket.local_addr().await.expect("addr").port(), j.port());
        assert!(!socket.ptr_eq(&server.primary));
        let got = server.stream.read().await.expect("server read");
        assert_eq!(got.as_deref(), Some(&b"c2"[..]));
        server.stream.write(b"s2").await.expect("server write");
    })
    .await
    .unwrap_or_else(|_| panic!("no arrival on the birthday socket within 5 s"));
    let answer = client.task.await.expect("client task").expect("client io");
    assert_eq!(answer.as_deref(), Some(&b"s2"[..]));
}

async fn once_scenario() {
    let (mut server, client_sock, rt, j) = punched_server().await;
    let _client = start(&mut server, &client_sock, rt, j).await;
    // The client repeats its SYN at 1 s and 3 s; only the first copy may be reported.
    let first = tokio::time::timeout(Duration::from_secs(5), server.irx.recv()).await;
    assert!(
        matches!(first, Ok(Some(Internal::Arrived { .. }))),
        "no arrival within 5 s"
    );
    let second = tokio::time::timeout(Duration::from_millis(1500), server.irx.recv()).await;
    assert!(second.is_err(), "a retransmitted SYN gave a second arrival");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn c2_child() {
    let Ok(scenario) = std::env::var("C2_SCENARIO") else {
        return;
    };
    match scenario.as_str() {
        "follow" => follow_scenario().await,
        "once" => once_scenario().await,
        other => panic!("unknown C2_SCENARIO {other}"),
    }
}

#[test]
fn c2_stream_follows_the_birthday_socket_that_got_the_syn() {
    run_child("follow");
}

#[test]
fn c2_retransmitted_syn_copies_give_one_arrival() {
    run_child("once");
}
