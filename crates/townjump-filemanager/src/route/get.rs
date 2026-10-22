// JS: livefiles/index.js:handleGetRequest (:93-110) and serveFile (:581-670).
use super::{Outcome, header_value_ok, listing::render_listing, plain, safe};
use crate::codec::{
    jspath,
    mime::content_type,
    range::{RangePlan, plan_range},
};
use crate::http::response::{Body, Response};
use crate::opts::Config;
use std::io::{ErrorKind, SeekFrom};
use tokio::io::AsyncSeekExt;

/// `Content-Disposition` value as Node 24 will write it (`processHeader`): with a nonzero
/// `Content-Length` the value first goes through `Buffer.from(v, 'latin1')` (char -> low byte),
/// then as a string (utf8, errors replaced) it is checked against the regexp `[^\t\x20-\x7e\x80-\xff]`.
/// `None` — Node throws ERR_INVALID_CHAR.
fn disposition_value(name: &str, len: u64) -> Option<String> {
    let v = format!("attachment; filename=\"{name}\"");
    let v = if len > 0 {
        let bytes: Vec<u8> = v.chars().map(|c| c as u32 as u8).collect();
        String::from_utf8_lossy(&bytes).into_owned()
    } else {
        v
    };
    header_value_ok(&v).then_some(v)
}

const BAD_CHAR: &str =
    "TypeError [ERR_INVALID_CHAR]: Invalid character in header content [\"Content-Disposition\"]";

pub(crate) async fn handle_get(
    cfg: &Config,
    full: &str,
    url: &str,
    range: Option<&str>,
) -> Outcome {
    if let Some(root) = &cfg.safe_root
        && !safe::confined(root, full).await
    {
        return plain(403, "Forbidden");
    }
    let meta = match tokio::fs::metadata(full).await {
        Ok(m) => m,
        Err(e) if e.kind() == ErrorKind::NotFound => return plain(404, "File Not Found"),
        Err(_) => return plain(500, "Internal Server Error"),
    };
    if meta.is_dir() {
        return render_listing(cfg, full, url).await;
    }
    if meta.is_file() {
        return serve_file(full, range, cfg.safe()).await;
    }
    Outcome::Hang
}

async fn serve_file(full: &str, range: Option<&str>, safe: bool) -> Outcome {
    let ext = jspath::extname(full).to_lowercase();
    let ctype = content_type(&ext);
    let size = match tokio::fs::metadata(full).await {
        Ok(m) => m.len(),
        Err(_) => return plain(500, "Error reading file."),
    };
    let (start, len, partial) = match plan_range(range, size) {
        RangePlan::Full => (0, size, None),
        RangePlan::Partial { start, end } => (start, end - start + 1, Some((start, end))),
        RangePlan::Unsatisfiable => return unsatisfiable(size),
        RangePlan::Crash(m) => return Outcome::Crash(m),
    };
    let name = jspath::basename(full);
    let disposition = if safe {
        safe::disposition(&name)
    } else {
        match disposition_value(&name, len) {
            Some(d) => d,
            None => return Outcome::Crash(BAD_CHAR.to_owned()),
        }
    };
    let Ok(mut file) = tokio::fs::File::open(full).await else {
        return Outcome::Hang;
    };
    if start > 0 && file.seek(SeekFrom::Start(start)).await.is_err() {
        return Outcome::Hang;
    }
    let headers = file_headers(partial, size, len, ctype, disposition);
    let status = if partial.is_some() { 206 } else { 200 };
    Outcome::Reply(Response {
        status,
        reason: None,
        headers,
        body: Body::File { file, len },
    })
}

fn file_headers(
    partial: Option<(u64, u64)>,
    size: u64,
    len: u64,
    ctype: &str,
    disposition: String,
) -> Vec<(String, String)> {
    let h = |k: &str, v: String| (k.to_owned(), v);
    let mut out = Vec::new();
    if let Some((s, e)) = partial {
        out.push(h("Content-Range", format!("bytes {s}-{e}/{size}")));
        out.push(h("Accept-Ranges", "bytes".to_owned()));
        out.push(h("Content-Length", len.to_string()));
        out.push(h("Content-Type", ctype.to_owned()));
    } else {
        out.push(h("Content-Length", len.to_string()));
        out.push(h("Content-Type", ctype.to_owned()));
        out.push(h("Accept-Ranges", "bytes".to_owned()));
    }
    out.push(h("Content-Disposition", disposition));
    out
}

fn unsatisfiable(size: u64) -> Outcome {
    Outcome::Reply(Response {
        status: 416,
        reason: None,
        headers: vec![("Content-Range".to_owned(), format!("bytes */{size}"))],
        body: Body::Empty,
    })
}

#[cfg(test)]
#[path = "get_tests.rs"]
mod tests;
