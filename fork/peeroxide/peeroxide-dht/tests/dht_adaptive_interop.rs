//! Live interop test: an adaptive Rust node ↔ a Node.js dht-rpc testnet.
//!
//! A firewalled Rust node starts ephemeral, promotes itself through PING_NAT, and must then
//! appear in the routing tables of the JS nodes (JS never stores an ephemeral node).

mod common;

use std::io::{BufRead, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{spawn_adaptive, within};
use peeroxide_dht::peer::peer_id;
use serde_json::Value;

fn node_testnet_path() -> String {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    format!("{manifest}/../tests/node/dht-rpc-testnet.js")
}

struct JsPeer {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    lines: Arc<Mutex<Vec<String>>>,
    _reader_thread: std::thread::JoinHandle<()>,
    next_id: u32,
}

impl JsPeer {
    fn spawn() -> Self {
        let mut child = Command::new("node")
            .arg(node_testnet_path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("failed to spawn node dht-rpc-testnet.js");

        let stdout = child.stdout.take().expect("no stdout");
        let stdin = child.stdin.take().expect("no stdin");
        let lines: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let lines_clone = lines.clone();

        let reader_thread = std::thread::spawn(move || {
            let reader = std::io::BufReader::new(stdout);
            for line in reader.lines().map_while(Result::ok) {
                lines_clone.lock().unwrap().push(line);
            }
        });

        Self {
            child,
            stdin,
            lines,
            _reader_thread: reader_thread,
            next_id: 1,
        }
    }

    fn read_ready_port(&self) -> u16 {
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        loop {
            {
                let lines = self.lines.lock().unwrap();
                if let Some(line) = lines.first() {
                    let v: Value =
                        serde_json::from_str(line).expect("invalid JSON from JS testnet");
                    assert!(
                        v["ready"].as_bool().unwrap_or(false),
                        "JS testnet not ready"
                    );
                    return v["port"].as_u64().expect("missing port") as u16;
                }
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for JS testnet ready"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn send_cmd(&mut self, cmd: serde_json::Value) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        let mut msg = cmd;
        msg["id"] = serde_json::json!(id);
        let line = serde_json::to_string(&msg).unwrap() + "\n";
        self.stdin
            .write_all(line.as_bytes())
            .expect("write to JS stdin");
        self.stdin.flush().expect("flush JS stdin");
        id
    }

    fn wait_reply(&self, id: u32) -> Value {
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        loop {
            {
                let lines = self.lines.lock().unwrap();
                for line in lines.iter() {
                    if let Ok(v) = serde_json::from_str::<Value>(line) {
                        if v["id"].as_u64() == Some(id as u64) {
                            return v;
                        }
                    }
                }
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for JS reply id={id}"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Ids (hex) in the union of the JS routing tables.
    fn table_ids(&mut self) -> Vec<String> {
        let id = self.send_cmd(serde_json::json!({ "cmd": "table" }));
        let reply = self.wait_reply(id);
        let nodes = reply["nodes"].as_array().cloned().unwrap_or_default();
        nodes
            .iter()
            .filter_map(|n| n["id"].as_str().map(String::from))
            .collect()
    }
}

impl Drop for JsPeer {
    fn drop(&mut self) {
        let _ = self.stdin.write_all(b"{\"cmd\":\"shutdown\",\"id\":0}\n");
        let _ = self.stdin.flush();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// Multi-thread: the JS helpers block, and the Rust node must keep running meanwhile.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn promoted_rust_node_enters_js_tables() {
    within(async {
        let mut js = tokio::task::block_in_place(JsPeer::spawn);
        let bs_port = tokio::task::block_in_place(|| js.read_ready_port());
        let node = spawn_adaptive(vec![format!("127.0.0.1:{bs_port}")], true, 2).await;
        let mut rx = node.handle.watch_mode();
        rx.wait_for(|m| !m.ephemeral).await.expect("node alive");
        let port = node.handle.local_port().await.expect("port");
        let want: String = peer_id("127.0.0.1", port)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        // JS adds the node after its first id-bearing request, i.e. the refresh after promotion.
        loop {
            let ids = tokio::task::block_in_place(|| js.table_ids());
            if ids.contains(&want) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    })
    .await;
}
