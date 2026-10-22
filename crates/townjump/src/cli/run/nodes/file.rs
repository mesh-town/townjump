// JS: none — a port extension; holesail JS starts every node with an empty table.
// Text format: `townjump-nodes 1`, `saved <unix seconds>`, then one `<ipv4>:<port>` per line, freshest first.
use std::collections::HashSet;
use std::net::SocketAddrV4;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const HEADER: &str = "townjump-nodes 1";

/// Upper bound on nodes read from a file.
pub(crate) const MAX_NODES: usize = 256;

/// Parses file text; any defect of the header, the stamp or the age gives an empty list.
pub(crate) fn parse(text: &str, now: u64, ttl: Duration) -> Vec<SocketAddrV4> {
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some(HEADER) {
        return Vec::new();
    }
    let Some(saved) = lines.next().and_then(saved_stamp) else {
        return Vec::new();
    };
    if now.saturating_sub(saved) > ttl.as_secs() {
        return Vec::new();
    }
    let mut seen = HashSet::new();
    let found = lines
        .filter_map(node_line)
        .filter(|addr| seen.insert(*addr));
    found.take(MAX_NODES).collect()
}

fn saved_stamp(line: &str) -> Option<u64> {
    line.trim().strip_prefix("saved ")?.trim().parse().ok()
}

fn node_line(line: &str) -> Option<SocketAddrV4> {
    let addr: SocketAddrV4 = line.trim().parse().ok()?;
    (addr.port() != 0).then_some(addr)
}

/// File text for `nodes` saved at `now` (unix seconds).
pub(crate) fn render(nodes: &[SocketAddrV4], now: u64) -> String {
    let mut text = format!("{HEADER}\nsaved {now}\n");
    for node in nodes {
        text.push_str(&format!("{node}\n"));
    }
    text
}

/// Reads and parses a file; an unreadable file gives an empty list.
pub(crate) fn load(path: &Path, now: u64, ttl: Duration) -> Vec<SocketAddrV4> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(&text, now, ttl),
        Err(_) => Vec::new(),
    }
}

/// Atomic write: a temporary file next to the target, then `rename`.
pub(crate) fn save(path: &Path, nodes: &[SocketAddrV4], now: u64) -> std::io::Result<()> {
    let tmp = temp_path(path);
    let written = std::fs::write(&tmp, render(nodes, now));
    let moved = written.and_then(|()| std::fs::rename(&tmp, path));
    if moved.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    moved
}

fn temp_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(format!(".{}.tmp", std::process::id()));
    PathBuf::from(name)
}

/// Current unix time in seconds.
pub(crate) fn unix_now() -> u64 {
    let since = SystemTime::now().duration_since(UNIX_EPOCH);
    since.map_or(0, |d| d.as_secs())
}

#[cfg(test)]
#[path = "file_tests.rs"]
mod tests;
