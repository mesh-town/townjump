// JS: interop/js/testnet.mjs (contract: the first stdout line is `TOWNJUMP_BOOTSTRAP=<host:port>[,…]`, runs until stdin is closed).
// Related: live network scenarios are in conformance/ (runs the fork adapter against this network). `std::process` only, no tokio.
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::Duration;

const PREFIX: &str = "TOWNJUMP_BOOTSTRAP=";
/// Start of node, loading hyperdht and the testnet bootstrap.
const START_WAIT: Duration = Duration::from_secs(60);

/// Live hyperdht testnet in a child node process; `stop` or `Drop` kill the process.
pub struct JsTestnet {
    child: Child,
    stdin: Option<ChildStdin>,
    bootstrap: Vec<String>,
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub(crate) fn script_path() -> PathBuf {
    repo_root().join("interop/js/testnet.mjs")
}

/// `TOWNJUMP_BOOTSTRAP=127.0.0.1:49737,[::1]:49738` → `["127.0.0.1:49737", "[::1]:49738"]`; a foreign line gives `None`.
pub fn parse_bootstrap_line(line: &str) -> Option<Vec<String>> {
    let list = line.trim().strip_prefix(PREFIX)?;
    let addrs: Vec<String> = list.split(',').map(str::to_owned).collect();
    addrs.iter().all(|addr| is_host_port(addr)).then_some(addrs)
}

fn is_host_port(addr: &str) -> bool {
    let Some((host, port)) = addr.rsplit_once(':') else {
        return false;
    };
    let digits = port.bytes().all(|b| b.is_ascii_digit());
    let usable = matches!(port.parse::<u16>(), Ok(number) if number != 0);
    !host.is_empty() && digits && usable
}

/// The first stdout line (empty means `EOF`), then stdout is read to the end: a closed pipe would kill node with SIGPIPE.
fn read_first_then_drain(stdout: ChildStdout, first: mpsc::Sender<io::Result<String>>) {
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    let read = reader.read_line(&mut line).map(|_| line);
    let _ = first.send(read);
    let _ = io::copy(&mut reader, &mut io::sink());
}

fn first_line(lines: &Receiver<io::Result<String>>, wait: Duration) -> io::Result<String> {
    let received = lines.recv_timeout(wait).map_err(|e| match e {
        RecvTimeoutError::Timeout => io::Error::new(
            io::ErrorKind::TimedOut,
            format!("testnet printed no first line within {wait:?}"),
        ),
        RecvTimeoutError::Disconnected => io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "the stdout reader thread broke off",
        ),
    });
    let line = received??;
    if line.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "the process closed stdout without printing a bootstrap line",
        ));
    }
    Ok(line)
}

/// Runs `cmd` and waits for the first line `TOWNJUMP_BOOTSTRAP=…`; on any error the process is killed (`Drop`).
pub(crate) fn spawn_read(mut cmd: Command, wait: Duration) -> io::Result<JsTestnet> {
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    let stdout = child.stdout.take().expect("stdout is set as a pipe");
    let stdin = child.stdin.take();
    let mut testnet = JsTestnet {
        child,
        stdin,
        bootstrap: Vec::new(),
    };
    let (sender, lines) = mpsc::channel();
    thread::spawn(move || read_first_then_drain(stdout, sender));
    let line = first_line(&lines, wait)?;
    testnet.bootstrap = parse_bootstrap_line(&line).ok_or_else(|| {
        let text = format!("the first line is not TOWNJUMP_BOOTSTRAP=host:port: {line:?}");
        io::Error::new(io::ErrorKind::InvalidData, text)
    })?;
    Ok(testnet)
}

impl JsTestnet {
    /// `node interop/js/testnet.mjs <n> --exit-on-stdin-eof`: bootstrap and `n` nodes on 127.0.0.1.
    pub fn start(n: usize) -> io::Result<JsTestnet> {
        let mut cmd = Command::new("node");
        cmd.arg(script_path())
            .arg(n.to_string())
            .arg("--exit-on-stdin-eof")
            .current_dir(repo_root());
        spawn_read(cmd, START_WAIT)
    }

    /// Bootstrap addresses `host:port` - the value for `TOWNJUMP_BOOTSTRAP` of child processes.
    pub fn bootstrap(&self) -> Vec<String> {
        self.bootstrap.clone()
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    pub fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Closes stdin, kills and awaits the process; a repeated call is harmless.
    pub fn stop(&mut self) {
        drop(self.stdin.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for JsTestnet {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
#[path = "js_testnet_tests.rs"]
mod tests;
