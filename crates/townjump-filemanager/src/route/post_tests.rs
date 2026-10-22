use super::*;
use crate::http::response::Body;
use townjump_jsval::JsValue;

fn cfg(root: &str, admin: bool) -> Config {
    Config {
        root: root.to_owned(),
        admin,
        username: JsValue::String("admin".into()),
        password: JsValue::String("admin".into()),
        host: JsValue::String("127.0.0.1".into()),
        port: JsValue::Number(0.0),
        on_fatal: None,
        safe_root: None,
        win_rules: false,
    }
}

static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Unique fixture root; the directory is removed on drop.
struct Temp(std::path::PathBuf);

impl std::ops::Deref for Temp {
    type Target = str;
    fn deref(&self) -> &str {
        self.0.to_str().unwrap()
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        if let Some(parent) = self.0.parent() {
            let _ = std::fs::remove_dir_all(parent);
        }
    }
}

fn temp(tag: &str) -> Temp {
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!("fm-post-{}-{tag}-{n}", std::process::id()));
    std::fs::create_dir_all(p.join("root")).unwrap();
    Temp(p.join("root"))
}

fn status_text(o: Outcome) -> (u16, String) {
    let Outcome::Reply(r) = o else {
        panic!("not Reply")
    };
    let text = match r.body {
        Body::Bytes(b) => String::from_utf8(b).unwrap(),
        _ => String::new(),
    };
    (r.status, text)
}

#[tokio::test]
async fn bad_form_is_400() {
    let root = temp("bad");
    let o = handle_post(&cfg(&root, true), "/", b"item_type=dir&name=x").await;
    assert_eq!(
        status_text(o),
        (400, "Bad Request: Missing or invalid form data.".into())
    );
}

#[tokio::test]
async fn user_cannot_create_folder() {
    let root = temp("user");
    let o = handle_post(&cfg(&root, false), "/", b"item_type=folder&name=x").await;
    assert_eq!(status_text(o).0, 403);
}

#[tokio::test]
async fn escape_is_403_and_nothing_created() {
    let root = temp("esc");
    let c = cfg(&root, true);
    let o = handle_post(&c, "/", b"item_type=file&name=../x").await;
    assert_eq!(status_text(o), (403, "Forbidden".into()));
    let o = handle_post(&c, "/", b"item_type=file&directory=..&name=x").await;
    assert_eq!(status_text(o).0, 403);
    assert!(!std::path::Path::new(&*root).join("../x").exists());
}

#[tokio::test]
async fn file_is_truncated_and_redirects() {
    let root = temp("trunc");
    let f = std::path::Path::new(&*root).join("a.txt");
    std::fs::write(&f, "data").unwrap();
    let o = handle_post(&cfg(&root, false), "/", b"item_type=file&name=a.txt").await;
    assert_eq!(status_text(o).0, 302);
    assert_eq!(std::fs::read(&f).unwrap().len(), 0);
}

#[tokio::test]
async fn win_names_denied_only_with_rules_in_safe_mode() {
    let root = temp("win");
    let mut c = cfg(&root, true);
    c.safe_root = Some(std::fs::canonicalize(&*root).unwrap());
    c.win_rules = true;
    let o = handle_post(&c, "/", b"item_type=file&name=nul.txt").await;
    assert_eq!(status_text(o), (403, "Forbidden".into()));
    let o = handle_post(&c, "/", b"item_type=folder&directory=CON&name=x").await;
    assert_eq!(status_text(o), (403, "Forbidden".into()));
    assert!(!std::path::Path::new(&*root).join("nul.txt").exists());
    assert!(!std::path::Path::new(&*root).join("CON").exists());
    c.win_rules = false;
    let o = handle_post(&c, "/", b"item_type=file&name=nul.txt").await;
    assert_eq!(status_text(o).0, 302);
    assert!(std::path::Path::new(&*root).join("nul.txt").exists());
}
