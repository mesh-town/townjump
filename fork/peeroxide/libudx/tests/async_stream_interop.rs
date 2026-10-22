#![deny(clippy::all)]

//! Close compatibility with JS (`udx-native`): DESTROY from a Rust peer must
//! give the JS side an `ECONNRESET` error, not an end of stream (`end`).

mod common;

use common::{create_runtime, node_script_path, with_timeout};
use std::io::{BufRead, BufReader, Write};
use std::net::SocketAddr;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

/// Kills the JS process on test exit, including on panic.
struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn spawn_reset_probe() -> (
    ChildGuard,
    BufReader<ChildStdout>,
    ChildStdin,
    u16,
    u32,
    u32,
) {
    let mut child = Command::new("node")
        .arg(node_script_path("udx-reset-probe.js"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn node");

    let stdout = child.stdout.take().expect("stdout");
    let stdin = child.stdin.take().expect("stdin");

    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader.read_line(&mut line).expect("read handshake");
    let info: serde_json::Value = serde_json::from_str(line.trim()).expect("parse handshake");
    let port = info["port"].as_u64().expect("port") as u16;
    let local_id = info["localId"].as_u64().expect("localId") as u32;
    let remote_id = info["remoteId"].as_u64().expect("remoteId") as u32;

    (ChildGuard(child), reader, stdin, port, local_id, remote_id)
}

async fn connect_to_probe(
    runtime: &libudx::UdxRuntime,
    node_port: u16,
    node_local_id: u32,
    node_remote_id: u32,
    mut stdin: ChildStdin,
    reader: &mut BufReader<ChildStdout>,
) -> (libudx::UdxStream, libudx::UdxSocket) {
    let socket = runtime.create_socket().await.expect("create socket");
    socket
        .bind("127.0.0.1:0".parse::<SocketAddr>().expect("parse"))
        .await
        .expect("bind");
    let local_addr = socket.local_addr().await.expect("local_addr");
    let rust_port = local_addr.port();

    let handshake = serde_json::json!({ "port": rust_port });
    writeln!(stdin, "{handshake}").expect("write handshake");
    drop(stdin);

    let mut ready_line = String::new();
    reader.read_line(&mut ready_line).expect("read ready");
    let ready: serde_json::Value = serde_json::from_str(ready_line.trim()).expect("parse ready");
    assert!(ready["ready"].as_bool().unwrap_or(false), "node not ready");

    let stream = runtime
        .create_stream(node_remote_id)
        .await
        .expect("create stream");
    let node_addr: SocketAddr = format!("127.0.0.1:{node_port}")
        .parse()
        .expect("parse addr");
    stream
        .connect(&socket, node_local_id, node_addr)
        .await
        .expect("connect");

    (stream, socket)
}

/// stdout lines of the JS process (after the handshake) go to a channel so they can be awaited with a timeout.
fn forward_lines(reader: BufReader<ChildStdout>) -> UnboundedReceiver<String> {
    let (tx, rx) = unbounded_channel();
    std::thread::spawn(move || {
        for line in reader.lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    rx
}

/// Waits for the first JS line for 5 s and requires exactly `{"error":"ECONNRESET"}`.
async fn expect_js_econnreset(lines: &mut UnboundedReceiver<String>) {
    let line = tokio::time::timeout(Duration::from_secs(5), lines.recv())
        .await
        .expect("JS did not print an event within 5 s")
        .expect("JS stdout closed without events");
    assert_eq!(
        line.trim(),
        r#"{"error":"ECONNRESET"}"#,
        "JS peer must receive ECONNRESET"
    );
}

#[tokio::test]
async fn destroy_resets_js_peer() {
    let _ = tracing_subscriber::fmt::try_init();

    with_timeout(Duration::from_secs(15), async {
        let (_child, mut reader, stdin, port, local_id, remote_id) = spawn_reset_probe();
        let runtime = create_runtime();
        let (stream, _socket) =
            connect_to_probe(&runtime, port, local_id, remote_id, stdin, &mut reader).await;
        let mut lines = forward_lines(reader);

        stream.destroy().await.expect("destroy");

        expect_js_econnreset(&mut lines).await;
    })
    .await;
}

#[tokio::test]
async fn drop_resets_js_peer() {
    let _ = tracing_subscriber::fmt::try_init();

    with_timeout(Duration::from_secs(15), async {
        let (_child, mut reader, stdin, port, local_id, remote_id) = spawn_reset_probe();
        let runtime = create_runtime();
        let (stream, _socket) =
            connect_to_probe(&runtime, port, local_id, remote_id, stdin, &mut reader).await;
        let mut lines = forward_lines(reader);

        let mut adapter = stream.into_async_stream();
        adapter.write_all(b"x").await.expect("write");
        adapter.flush().await.expect("flush");
        drop(adapter);

        expect_js_econnreset(&mut lines).await;
    })
    .await;
}
