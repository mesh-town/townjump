// JS: livefiles/index.js:handlePostRequest (:123-189), createFolder/createFile (:676-700).
use super::{Outcome, found_500, header_value_ok, plain, safe};
use crate::codec::{
    jspath,
    query::{QsValue, get, parse_query},
};
use crate::http::response::{Body, Response};
use crate::opts::Config;

pub(crate) async fn handle_post(cfg: &Config, url: &str, body: &[u8]) -> Outcome {
    let q = parse_query(&String::from_utf8_lossy(body));
    let (folder, name) = match fields(&q) {
        Some(f) => f,
        None => return plain(400, "Bad Request: Missing or invalid form data."),
    };
    if folder && !cfg.admin {
        return plain(403, "Forbidden: Only admin users can create folders.");
    }
    let dir = match get(&q, "directory") {
        Some(QsValue::Many(_)) => return plain(500, "Internal Server Error"),
        Some(QsValue::One(d)) if !d.is_empty() => d.as_str(),
        _ => ".",
    };
    let new = jspath::join(&[&cfg.root, dir, name]);
    if jspath::escapes_root(&cfg.root, &new) || safe::win_name_denied(cfg, &new) {
        return plain(403, "Forbidden");
    }
    if let Some(root) = &cfg.safe_root
        && !safe::confined(root, &new).await
    {
        return plain(403, "Forbidden");
    }
    create(&new, folder, url, cfg.safe()).await
}

/// (this is a folder, name) or None if the form is malformed.
fn fields(q: &[(String, QsValue)]) -> Option<(bool, &str)> {
    let folder = match get(q, "item_type") {
        Some(QsValue::One(t)) if t == "folder" => true,
        Some(QsValue::One(t)) if t == "file" => false,
        _ => return None,
    };
    match get(q, "name") {
        Some(QsValue::One(n)) if !n.is_empty() => Some((folder, n.as_str())),
        _ => None,
    }
}

async fn create(new: &str, folder: bool, url: &str, safe: bool) -> Outcome {
    let (done, fail) = if folder {
        (
            tokio::fs::create_dir_all(new).await,
            "Error creating folder.",
        )
    } else if safe {
        (safe::create_new(new).await, "Error creating file.")
    } else {
        (tokio::fs::write(new, b"").await, "Error creating file.")
    };
    if let Err(e) = done {
        if safe && !folder && e.kind() == std::io::ErrorKind::AlreadyExists {
            return plain(409, "Conflict: File already exists.");
        }
        return plain(500, fail);
    }
    let location = if safe {
        safe::location(url)
    } else {
        url.to_owned()
    };
    if !safe && !header_value_ok(url) {
        // JS: writeHead(302, {Location}) throws ERR_INVALID_CHAR but manages to write
        // statusMessage 'Found'; the catch writes writeHead(500) with the same reason -> `500 Found`.
        return found_500(fail);
    }
    Outcome::Reply(Response {
        status: 302,
        reason: None,
        headers: vec![("Location".to_owned(), location)],
        body: Body::Empty,
    })
}

#[cfg(test)]
#[path = "post_tests.rs"]
mod tests;
