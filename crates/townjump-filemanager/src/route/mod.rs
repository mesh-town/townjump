// JS: livefiles/index.js:123-189 (request handler: URL parsing, authorization, method).
use crate::codec::{base64::node_base64, jspath, text::ct_eq, uri::decode_uri_component};
use crate::http::head::RequestHead;
use crate::http::response::{Body, Response};
use crate::opts::Config;
use townjump_jsval::JsValue;

pub(crate) mod get;
pub(crate) mod listing;
pub(crate) mod post;
pub(crate) mod safe;

#[cfg(test)]
#[path = "admit_tests.rs"]
mod admit_tests;

pub(crate) enum Outcome {
    Reply(Response),
    ReplyClose(Response),
    Hang,
    Crash(String),
}

pub(crate) enum Step {
    Done(Outcome),
    Get { full: String, url: String },
    Post { url: String },
}

pub(crate) enum Auth {
    Pass,
    Deny,
    Crash(String),
}

const NO_ARG: &str = "TypeError [ERR_INVALID_ARG_TYPE]: The first argument must be of type string or an instance of Buffer, ArrayBuffer, or Array or an Array-like Object. Received undefined";

/// `text/plain` response with a string body.
pub(crate) fn plain(status: u16, text: &str) -> Outcome {
    let headers = vec![("Content-Type".to_owned(), "text/plain".to_owned())];
    Outcome::Reply(Response {
        status,
        reason: None,
        headers,
        body: Body::Bytes(text.as_bytes().to_vec()),
    })
}

/// `500` with the reason `Found` (see `post.rs::create`).
pub(crate) fn found_500(text: &str) -> Outcome {
    let Outcome::Reply(mut r) = plain(500, text) else {
        unreachable!("plain is always Reply")
    };
    r.reason = Some("Found");
    Outcome::Reply(r)
}

/// Characters that Node lets into a header value.
pub(crate) fn header_value_ok(v: &str) -> bool {
    v.chars()
        .all(|c| matches!(c, '\t' | ' '..='~' | '\u{80}'..='\u{ff}'))
}

pub(crate) fn admit(cfg: &Config, head: &RequestHead) -> Step {
    let url = match decode_uri_component(&head.target) {
        Ok(u) => u,
        Err(_) => return Step::Done(Outcome::Crash("URIError: URI malformed".to_owned())),
    };
    let full = jspath::join(&[&cfg.root, &url]);
    match authenticate(cfg, head.headers.get("authorization")) {
        Auth::Deny => return Step::Done(unauthorized()),
        Auth::Crash(m) => return Step::Done(Outcome::Crash(m)),
        Auth::Pass => {}
    }
    if cfg.safe()
        && let Some(o) = safe::screen(cfg, head)
    {
        return Step::Done(o);
    }
    match head.method.as_str() {
        "GET" if jspath::escapes_root(&cfg.root, &full) || safe::win_name_denied(cfg, &full) => {
            Step::Done(plain(403, "Forbidden"))
        }
        "GET" => Step::Get { full, url },
        "POST" => Step::Post { url },
        _ => Step::Done(Outcome::Hang),
    }
}

fn unauthorized() -> Outcome {
    let headers = vec![(
        "WWW-Authenticate".to_owned(),
        "Basic realm=\"Filemanager\"".to_owned(),
    )];
    Outcome::Reply(Response {
        status: 401,
        reason: None,
        headers,
        body: Body::Bytes(b"Authentication required.".to_vec()),
    })
}

pub(crate) fn authenticate(cfg: &Config, header: Option<&str>) -> Auth {
    let header = match header {
        Some(h) if !h.is_empty() => h,
        _ => return Auth::Deny,
    };
    let Some(encoded) = header.split(' ').nth(1) else {
        return Auth::Crash(NO_ARG.to_owned());
    };
    let decoded = String::from_utf8_lossy(&node_base64(encoded)).into_owned();
    let mut parts = decoded.split(':');
    let user = parts.next();
    let pass = parts.next();
    // Both comparisons always run, no early exit via `&&`.
    let (u_ok, p_ok) = (same(&cfg.username, user), same(&cfg.password, pass));
    if u_ok & p_ok { Auth::Pass } else { Auth::Deny }
}

/// `===` with a string: only `JsValue::String` with the same text.
fn same(want: &JsValue, got: Option<&str>) -> bool {
    matches!((want, got), (JsValue::String(w), Some(g)) if ct_eq(w.as_bytes(), g.as_bytes()))
}
