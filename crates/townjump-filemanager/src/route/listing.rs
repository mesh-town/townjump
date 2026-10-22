// JS: livefiles/index.js:listDirectory (:191-305), calculateDirectorySize (:158-190), getDirectoryOptions (:749-784).
use super::{Outcome, plain, safe};
use crate::codec::{
    jspath,
    text::{escape_html, format_bytes, replace_first},
};
use crate::http::response::{Body, Response};
use crate::opts::Config;
use crate::page;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;

struct Entry {
    name: String,
    is_dir: bool,
    is_link: bool,
}

pub(crate) async fn render_listing(cfg: &Config, full: &str, url: &str) -> Outcome {
    let Ok(entries) = read_entries(full).await else {
        return plain(500, "Internal Server Error");
    };
    let mut rows = String::new();
    let safe_root = cfg.safe_root.as_deref();
    for e in &entries {
        let path = jspath::join(&[full, &e.name]);
        if safe::win_name_denied(cfg, &path) {
            continue;
        }
        if let Some(root) = safe_root
            && e.is_link
            && !safe::confined(root, &path).await
        {
            continue;
        }
        if let Some(row) = entry_row(full, url, e, safe_root).await {
            rows.push_str(&row);
        }
    }
    let form = if cfg.admin {
        page::form(&safe::html(cfg.safe(), url), &dir_options(cfg).await)
    } else {
        String::new()
    };
    let html = page::page(&escape_html(url), &rows, &form);
    let headers = vec![("Content-Type".to_owned(), "text/html".to_owned())];
    Outcome::Reply(Response {
        status: 200,
        reason: None,
        headers,
        body: Body::Bytes(html.into_bytes()),
    })
}

/// Entries sorted by name bytes, directories first; type without following links.
async fn read_entries(dir: &str) -> std::io::Result<Vec<Entry>> {
    let mut rd = tokio::fs::read_dir(dir).await?;
    let mut all = Vec::new();
    while let Some(e) = rd.next_entry().await? {
        let Ok(t) = e.file_type().await else {
            continue;
        };
        let is_dir = t.is_dir();
        let is_link = t.is_symlink();
        let name = e.file_name().to_string_lossy().into_owned();
        all.push(Entry {
            name,
            is_dir,
            is_link,
        });
    }
    all.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
    let (mut dirs, files): (Vec<Entry>, Vec<Entry>) = all.into_iter().partition(|e| e.is_dir);
    dirs.extend(files);
    Ok(dirs)
}

async fn entry_row(full: &str, url: &str, e: &Entry, safe_root: Option<&Path>) -> Option<String> {
    let path = jspath::join(&[full, &e.name]);
    if !readable(&path, e.is_dir).await {
        return None;
    }
    let href = jspath::join(&[url, &e.name]);
    let href = safe::html(safe_root.is_some(), &href);
    let size = if e.is_dir {
        dir_size(&path, safe_root).await
    } else {
        tokio::fs::metadata(&path).await.ok()?.len()
    };
    let (icon, button) = if e.is_dir {
        (
            page::FOLDER_ICON,
            format!("<a class=\"open--btn\" href=\"{href}\">Enter</a>"),
        )
    } else {
        (
            page::FILE_ICON,
            format!("<a href=\"{href}\" download>Download</a>"),
        )
    };
    let size = format_bytes(size as f64);
    Some(page::row(
        icon,
        &href,
        &escape_html(&e.name),
        &button,
        &size,
    ))
}

/// `access(R_OK)`: a directory is readable, a regular file opens. FIFO, socket and device
/// are not opened (open on a FIFO blocks until a writer appears): permission bits decide.
async fn readable(path: &str, is_dir: bool) -> bool {
    if is_dir {
        return tokio::fs::read_dir(path).await.is_ok();
    }
    let Ok(meta) = tokio::fs::metadata(path).await else {
        return false;
    };
    if meta.is_file() {
        return tokio::fs::File::open(path).await.is_ok();
    }
    other_readable(&meta)
}

#[cfg(unix)]
fn other_readable(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o444 != 0
}

#[cfg(not(unix))]
fn other_readable(_meta: &std::fs::Metadata) -> bool {
    true
}

/// Sum of sizes of direct non-directory children; any directory read error gives 0.
pub(crate) async fn dir_size(path: &str, safe_root: Option<&Path>) -> u64 {
    let Ok(mut rd) = tokio::fs::read_dir(path).await else {
        return 0;
    };
    let mut total = 0;
    loop {
        let Ok(Some(e)) = rd.next_entry().await else {
            break;
        };
        match e.file_type().await {
            Ok(t) if !t.is_dir() => {
                let outside = match safe_root {
                    Some(root) if t.is_symlink() => {
                        !safe::confined(root, &e.path().to_string_lossy()).await
                    }
                    _ => false,
                };
                if outside {
                    continue;
                }
            }
            _ => continue,
        }
        if let Ok(m) = tokio::fs::metadata(e.path()).await {
            total += m.len();
        }
    }
    total
}

pub(crate) async fn dir_options(cfg: &Config) -> String {
    traverse(cfg.root.clone(), cfg.root.clone(), 0, cfg.safe()).await
}

type Boxed = Pin<Box<dyn Future<Output = String> + Send>>;

/// Walk of subdirectories, depth < 5; inaccessible directories are skipped.
fn traverse(root: String, dir: String, depth: usize, safe: bool) -> Boxed {
    Box::pin(async move {
        let Ok(entries) = read_entries(&dir).await else {
            return String::new();
        };
        let mut out = String::new();
        for e in entries.iter().filter(|e| e.is_dir) {
            let item = jspath::join(&[&dir, &e.name]);
            let shown = replace_first(&item, &root, "");
            let indent = "&nbsp;".repeat(depth * 4);
            out.push_str(&format!(
                "<option value=\"{}\">{indent}{}</option>",
                safe::html(safe, &shown),
                safe::html(safe, &e.name)
            ));
            if depth < 5 {
                out.push_str(&traverse(root.clone(), item, depth + 1, safe).await);
            }
        }
        out
    })
}

#[cfg(test)]
#[path = "listing_tests.rs"]
mod tests;
