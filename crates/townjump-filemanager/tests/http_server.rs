//! Whole server: root escape, methods, keepalive, bind error, `Filemanager::new`
//! Authentication is in `http_auth.rs`, listing and files in `http_files.rs`,
//! POST — `http_post.rs`.
mod support;

use std::time::Duration;
use support::{ADMIN_AUTH, Conn, Reply, Setup, TempRoot, crash_texts, crashes, request, setup};
use townjump_filemanager::{Filemanager, FilemanagerError, FilemanagerOpts};
use townjump_jsval::JsValue;

mod get_with_dot_dot_path {
    use super::*;

    // Root escape: `GET /../` → 403 `Forbidden`, text/plain.
    #[tokio::test]
    async fn is_forbidden() {
        let site = setup(Setup::default()).await;

        let reply = site.get("/../").await;

        then_forbidden(&reply);
    }
}

mod get_with_percent_encoded_dot_dot_path {
    use super::*;

    // `GET /%2e%2e/` (the same `..` after decode) → 403 `Forbidden`.
    #[tokio::test]
    async fn is_forbidden() {
        let site = setup(Setup::default()).await;

        let reply = site.get("/%2e%2e/").await;

        then_forbidden(&reply);
    }
}

mod post_with_name_leading_out_of_root {
    use super::*;

    // POST `item_type=file&name=../x` → 403, no file created outside root.
    #[tokio::test]
    async fn is_forbidden_and_creates_nothing_outside() {
        let site = setup(Setup::default()).await;

        let reply = site.post("/", "item_type=file&name=../x").await;

        then_forbidden(&reply);
        assert!(!site.root.outside_exists("x"), "file created outside root");
    }
}

mod post_with_directory_dot_dot {
    use super::*;

    // POST `directory=..&name=x` (with item_type — otherwise 400) → 403, no file created outside root.
    #[tokio::test]
    async fn is_forbidden_and_creates_nothing_outside() {
        let site = setup(Setup::default()).await;

        let reply = site.post("/", "item_type=file&directory=..&name=x").await;

        then_forbidden(&reply);
        assert!(!site.root.outside_exists("x"), "file created outside root");
    }
}

mod put_method {
    use super::*;

    // Methods: `PUT /` → no byte within 500 ms, socket open.
    #[tokio::test]
    async fn gets_no_bytes_and_keeps_the_socket_open() {
        // given
        let site = setup(Setup::default()).await;
        let mut conn = Conn::open(site.addr).await;
        // when
        conn.send(&keep_alive("PUT", "/")).await;
        // then
        assert!(
            conn.stays_silent(Duration::from_millis(500)).await,
            "server answered or closed the socket"
        );
    }
}

mod two_gets_on_one_socket {
    use super::*;

    // Methods: keepalive — two GETs in a row on one socket → two responses.
    #[tokio::test]
    async fn get_two_replies() {
        // given
        let site = setup(Setup {
            files: &[("b.txt", "abc")],
            ..Default::default()
        })
        .await;
        let mut conn = Conn::open(site.addr).await;
        // when
        conn.send(&keep_alive("GET", "/b.txt")).await;
        let first = conn.reply().await;
        conn.send(&keep_alive("GET", "/b.txt")).await;
        let second = conn.reply().await;
        // then
        assert_eq!((first.status, second.status), (200, 200));
        assert_eq!(second.text(), "abc");
    }
}

mod idle_connection {
    use super::*;

    // Idle 6 s after the response → the server closed the socket (KEEP_ALIVE 5 s).
    #[tokio::test]
    #[ignore = "6 s idle"]
    async fn is_closed_by_server_after_keep_alive_timeout() {
        // given
        let site = setup(Setup::default()).await;
        let mut conn = Conn::open(site.addr).await;
        conn.send(&keep_alive("GET", "/")).await;
        assert_eq!(conn.reply().await.status, 200);
        // when
        tokio::time::sleep(Duration::from_secs(6)).await;
        // then
        assert!(
            conn.closed_within(Duration::from_secs(1)).await,
            "server did not close the idle socket"
        );
    }
}

mod dropped_server {
    use super::*;

    // Dropping without `close()` frees the port (Drop aborts the accept task).
    #[tokio::test]
    async fn frees_the_port() {
        let root = TempRoot::new();
        let (fm, addr) = support::start(FilemanagerOpts {
            path: Some(JsValue::String(root.path())),
            ..Default::default()
        })
        .await;
        drop(fm);
        let mut bound = false;
        for _ in 0..20 {
            if std::net::TcpListener::bind(addr).is_ok() {
                bound = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(bound, "port {addr} stayed busy after drop");
    }
}

mod busy_port {
    use super::*;

    // Bind: port busy → `ready()` Ok, the hook got the EADDRINUSE text.
    #[tokio::test]
    async fn is_reported_to_the_hook_while_ready_succeeds() {
        // given
        let (_holder, port) = occupied_port();
        let root = TempRoot::new();
        let (hook, log) = crashes();
        let mut fm = Filemanager::new(FilemanagerOpts {
            path: Some(JsValue::String(root.path())),
            port: Some(JsValue::Number(f64::from(port))),
            on_fatal: Some(hook),
            ..Default::default()
        })
        .expect("Filemanager::new");
        // when
        let ready = fm.ready().await;
        // then
        assert_eq!(ready, Ok(()));
        assert_eq!(
            crash_texts(&log).await,
            [format!(
                "Error: listen EADDRINUSE: address already in use 127.0.0.1:{port}"
            )]
        );
    }
}

mod new_without_usable_path {
    use super::*;

    // `new`: `path: None` → NoPath.
    #[test]
    fn none_is_no_path() {
        assert_eq!(rejected(None), Some(FilemanagerError::NoPath));
    }

    // A nonexistent directory path → NoPath.
    #[test]
    fn missing_directory_is_no_path() {
        let path = JsValue::String("/νετ/τακογο".into());

        assert_eq!(rejected(Some(path)), Some(FilemanagerError::NoPath));
    }

    // `Bool(true)` (the `--filemanager` flag without a value) → NoPath.
    #[test]
    fn bool_true_is_no_path() {
        assert_eq!(
            rejected(Some(JsValue::Bool(true))),
            Some(FilemanagerError::NoPath)
        );
    }

    // `new` requires a non-empty `JsValue::String` — an empty string → NoPath.
    #[test]
    fn empty_string_is_no_path() {
        let path = JsValue::String(String::new());

        assert_eq!(rejected(Some(path)), Some(FilemanagerError::NoPath));
    }

    // Error Display is `Error: INCORRECT OR NO PATH SPECIFIED`.
    #[test]
    fn error_text_matches_js() {
        let error = rejected(None).expect("NoPath");

        assert_eq!(error.to_string(), "Error: INCORRECT OR NO PATH SPECIFIED");
    }
}

/// 403, `Content-Type: text/plain`, body `Forbidden`.
fn then_forbidden(reply: &Reply) {
    assert_eq!(reply.status, 403);
    assert_eq!(reply.header("Content-Type"), Some("text/plain"));
    assert_eq!(reply.text(), "Forbidden");
}

/// Request `admin:admin` with an explicit `Connection: keep-alive`.
fn keep_alive(method: &str, target: &str) -> Vec<u8> {
    request(
        method,
        target,
        &[ADMIN_AUTH, ("Connection", "keep-alive")],
        b"",
    )
}

/// Listening socket on a free port; while the `TcpListener` is alive, the port is taken.
fn occupied_port() -> (std::net::TcpListener, u16) {
    let holder = std::net::TcpListener::bind("127.0.0.1:0").expect("take a port");
    let port = holder.local_addr().expect("local_addr").port();
    (holder, port)
}

/// `Filemanager::new` error for the given `path`.
fn rejected(path: Option<JsValue>) -> Option<FilemanagerError> {
    Filemanager::new(FilemanagerOpts {
        path,
        ..Default::default()
    })
    .err()
}
