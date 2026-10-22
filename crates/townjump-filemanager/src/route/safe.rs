// Port extension (absent in JS): the `--safe-mode` mode for `--filemanager`. All checks and texts of the mode.
use super::{Outcome, plain};
use crate::codec::jspath;
use crate::codec::text::escape_html;
use crate::http::head::RequestHead;
use crate::http::response::{Body, Response};
use crate::opts::Config;
use std::borrow::Cow;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};

/// Request checks after the password: Host -> method -> (POST) role -> (POST) Origin. `None` — let through.
pub(crate) fn screen(cfg: &Config, head: &RequestHead) -> Option<Outcome> {
    let host = head.headers.get("host");
    if host.is_some_and(|h| !host_allowed(h)) {
        return Some(plain(403, "Forbidden: Host not allowed."));
    }
    match head.method.as_str() {
        "GET" => return None,
        "POST" => {}
        _ => return Some(method_not_allowed()),
    }
    if !cfg.admin {
        return Some(plain(403, "Forbidden: Only admin users can create files."));
    }
    match head.headers.get("origin") {
        Some(o) if !same_origin(o, host) => Some(plain(403, "Forbidden: Cross-origin request.")),
        _ => None,
    }
}

/// Windows device names / trailing dots after the root are refused (only in `--safe-mode`).
pub(crate) fn win_name_denied(cfg: &Config, full: &str) -> bool {
    cfg.safe() && jspath::win_names_as(&cfg.root, full, cfg.win_rules)
}

/// `Host` admission (the whole header value): IP literal, a single name, `*.localhost`, `*.local`.
pub(crate) fn host_allowed(host: &str) -> bool {
    let name = if let Some(rest) = host.strip_prefix('[') {
        let Some((inner, tail)) = rest.split_once(']') else {
            return false;
        };
        return inner.parse::<Ipv6Addr>().is_ok() && port_ok(tail);
    } else {
        match host.split_once(':') {
            Some((n, port)) if port_digits(port) => n,
            Some(_) => return false,
            None => host,
        }
    };
    if name.parse::<Ipv4Addr>().is_ok() {
        return true;
    }
    let labels: Vec<&str> = name.split('.').collect();
    let chars_ok =
        |l: &&str| !l.is_empty() && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
    if !labels.iter().all(chars_ok) {
        return false;
    }
    let last = labels[labels.len() - 1];
    labels.len() == 1
        || last.eq_ignore_ascii_case("localhost")
        || last.eq_ignore_ascii_case("local")
}

fn port_digits(p: &str) -> bool {
    !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())
}

/// Tail after `]`: empty or `:<digits>`.
fn port_ok(tail: &str) -> bool {
    tail.is_empty() || tail.strip_prefix(':').is_some_and(port_digits)
}

/// Origin matches Host (case-insensitive); scheme `http`/`https`; `null` and anything else is rejected.
pub(crate) fn same_origin(origin: &str, host: Option<&str>) -> bool {
    let lower = origin.to_ascii_lowercase();
    let authority = lower
        .strip_prefix("http://")
        .or_else(|| lower.strip_prefix("https://"));
    match (authority, host) {
        (Some(a), Some(h)) => a == h.to_ascii_lowercase(),
        _ => false,
    }
}

/// Response instead of a JS crash: `400`, the connection is closed (`keep = false` at the caller).
pub(crate) fn bad_request() -> Response {
    Response {
        status: 400,
        reason: None,
        headers: vec![("Content-Type".to_owned(), "text/plain".to_owned())],
        body: Body::Bytes(b"Bad Request".to_vec()),
    }
}

pub(crate) fn method_not_allowed() -> Outcome {
    let headers = vec![
        ("Allow".to_owned(), "GET, POST".to_owned()),
        ("Content-Type".to_owned(), "text/plain".to_owned()),
    ];
    Outcome::Reply(Response {
        status: 405,
        reason: None,
        headers,
        body: Body::Bytes(b"Method Not Allowed".to_vec()),
    })
}

/// The path (or its nearest existing ancestor) after link resolution lies inside the canonical root.
pub(crate) async fn confined(root: &Path, full: &str) -> bool {
    let mut path = PathBuf::from(full);
    loop {
        if path.as_os_str().is_empty() {
            path = PathBuf::from(".");
        }
        match tokio::fs::canonicalize(&path).await {
            Ok(real) => return real.starts_with(root),
            // Climbing is allowed only for "does not exist"; any other failure is a rejection.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && path != Path::new(".") => {
                if !path.pop() {
                    return false;
                }
            }
            Err(_) => return false,
        }
    }
}

/// HTML escaping only when `on`.
pub(crate) fn html(on: bool, s: &str) -> Cow<'_, str> {
    if on {
        Cow::Owned(escape_html(s))
    } else {
        Cow::Borrowed(s)
    }
}

/// `attachment; filename="<ascii>"; filename*=UTF-8''<pct>` (RFC 6266 / 5987).
pub(crate) fn disposition(name: &str) -> String {
    let ascii: String = name
        .chars()
        .map(|c| match c {
            '"' | '\\' | ';' => '_',
            ' '..='~' => c,
            _ => '_',
        })
        .collect();
    let pct = pct_encode(name, |b| {
        b.is_ascii_alphanumeric() || b"!#$&+-.^_`|~".contains(&b)
    });
    format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{pct}")
}

/// `encodeURI` for a path: `%XX` (uppercase) for UTF-8 bytes except unescaped ones; `#` and `?` are encoded too.
pub(crate) fn location(url: &str) -> String {
    pct_encode(url, |b| {
        b.is_ascii_alphanumeric() || b";,/:@&=+$-_.!~*'()".contains(&b)
    })
}

fn pct_encode(s: &str, keep: impl Fn(u8) -> bool) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if keep(b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `O_CREAT|O_EXCL`: an existing entry (including a dangling link) -> `AlreadyExists`.
pub(crate) async fn create_new(path: &str) -> std::io::Result<()> {
    tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .await
        .map(|_| ())
}

#[cfg(test)]
#[path = "safe_tests.rs"]
mod tests;
