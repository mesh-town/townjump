//! Live interop test: closing a Rust server withdraws its record from Node.js nodes.
//!
//! A Rust server announces in a Node.js testnet; a Node.js peer finds the record with
//! FIND_PEER; after `Server::close` the same query finds nothing. This checks that the
//! UNANNOUNCE the Rust side sends (token from FIND_PEER, signature) is accepted by the
//! reference implementation.

use std::io::{BufRead, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use libudx::UdxRuntime;
use peeroxide_dht::hyperdht::{HyperDhtConfig, KeyPair, spawn};
use peeroxide_dht::rpc::DhtConfig;
use peeroxide_dht::server::{Server, ServerOpts};
use serde_json::Value;

fn node_peer_path() -> String {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    format!("{manifest}/../tests/node/hyperdht-unannounce-peer.js")
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
            .arg(node_peer_path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("failed to spawn node hyperdht-unannounce-peer.js");

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
                    let v: Value = serde_json::from_str(line).expect("invalid JSON from JS peer");
                    assert!(v["ready"].as_bool().unwrap_or(false), "JS peer not ready");
                    return v["port"].as_u64().expect("missing port") as u16;
                }
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for JS peer ready"
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

    fn find_peer(&mut self, public_key_hex: &str) -> bool {
        let id =
            self.send_cmd(serde_json::json!({ "cmd": "find_peer", "public_key": public_key_hex }));
        let reply = self.wait_reply(id);
        assert!(
            reply["ok"].as_bool().unwrap_or(false),
            "JS find_peer failed: {reply}"
        );
        reply["found"].as_bool().expect("missing found")
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

fn to_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[tokio::test]
async fn rust_server_close_removes_the_record_from_js_nodes() {
    let _ = tracing_subscriber::fmt::try_init();

    let result = tokio::time::timeout(Duration::from_secs(60), run_interop()).await;

    match result {
        Ok(Ok(())) => {}
        Ok(Err(e)) => panic!("interop test failed: {e}"),
        Err(_) => panic!("interop test timed out after 60s"),
    }
}

async fn run_interop() -> Result<(), Box<dyn std::error::Error>> {
    let mut js = JsPeer::spawn();
    let bs_port = js.read_ready_port();

    let runtime = UdxRuntime::new()?;

    let mut dht_config = DhtConfig::default();
    dht_config.bootstrap = vec![format!("127.0.0.1:{bs_port}")];
    dht_config.port = 0;
    dht_config.host = "127.0.0.1".to_string();
    dht_config.firewalled = false;

    let mut config = HyperDhtConfig::default();
    config.dht = dht_config;

    let (task, handle, server_rx) = spawn(&runtime, config).await?;
    handle.bootstrapped().await?;

    let kp = KeyPair::generate();
    let pk_hex = to_hex(&kp.public_key);

    let (server, _incoming) = Server::new(
        handle.clone(),
        server_rx,
        UdxRuntime::shared(runtime.handle()),
        ServerOpts::default(),
    );
    server.listen(kp).await?;

    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        js.find_peer(&pk_hex),
        "record never reached JS nodes after listen"
    );

    server.close().await?;

    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        !js.find_peer(&pk_hex),
        "JS nodes still hold the record after Server::close"
    );

    handle.destroy().await?;
    let _ = task.await;
    drop(js);
    drop(runtime);

    Ok(())
}
