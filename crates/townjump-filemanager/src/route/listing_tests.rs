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

/// Unique fixture directory; removed on drop.
struct Fixture(std::path::PathBuf);

impl Fixture {
    fn root(&self) -> String {
        self.0.join("root").to_string_lossy().into_owned()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fixture() -> Fixture {
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!("fm-list-{}-{n}", std::process::id()));
    let root = p.join("root");
    std::fs::create_dir_all(root.join("a")).unwrap();
    std::fs::write(root.join("a/x.bin"), [0u8; 10]).unwrap();
    std::fs::write(root.join("b.txt"), "abc").unwrap();
    Fixture(p)
}

async fn html(admin: bool) -> String {
    let fx = fixture();
    html_of(&fx.root(), admin).await
}

async fn html_of(root: &str, admin: bool) -> String {
    let Outcome::Reply(r) = render_listing(&cfg(root, admin), root, "/").await else {
        panic!("not Reply")
    };
    let Body::Bytes(b) = r.body else {
        panic!("not Bytes")
    };
    String::from_utf8(b).unwrap()
}

#[tokio::test]
async fn folders_first_with_sizes() {
    let h = html(false).await;
    let a = h.find("<a href=\"/a\">a</a>").unwrap();
    let b = h.find("<a href=\"/b.txt\">b.txt</a>").unwrap();
    assert!(a < b);
    assert!(h.contains("10 Bytes") && h.contains("3 Bytes"));
}

#[tokio::test]
async fn form_only_for_admin() {
    assert!(!html(false).await.contains("item_type"));
    assert!(html(true).await.contains("item_type"));
}

#[tokio::test]
async fn dir_size_counts_direct_files_only() {
    let fx = fixture();
    assert_eq!(dir_size(&fx.root(), None).await, 3);
    assert_eq!(dir_size("/nonexistent-fm-dir", None).await, 0);
}

/// JS: `fs.access(R_OK)` does not block on a FIFO, the entry lands in the list (index.js:243).
#[cfg(unix)]
#[tokio::test]
async fn listing_with_fifo_does_not_hang() {
    let fx = fixture();
    let root = fx.root();
    let fifo = std::path::Path::new(&root).join("pipe");
    let ok = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    assert!(ok, "mkfifo did not create a FIFO");
    let h = tokio::time::timeout(std::time::Duration::from_secs(2), html_of(&root, false))
        .await
        .expect("listing with a FIFO hung");
    assert!(h.contains("<a href=\"/pipe\">pipe</a>"));
}

#[tokio::test]
async fn win_unsafe_names_left_out_of_safe_listing() {
    let fx = fixture();
    let root = std::path::PathBuf::from(fx.root());
    // Create what the OS accepts; keep only names `read_dir` returns exactly as given
    // (Windows strips the trailing dot of `secret.txt.` and may refuse `CON`).
    for f in ["ok.txt", "nul.txt", "secret.txt."] {
        let _ = std::fs::write(root.join(f), "x");
    }
    let _ = std::fs::create_dir(root.join("CON"));
    let present: Vec<String> = std::fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    let names: Vec<&str> = ["ok.txt", "nul.txt", "secret.txt.", "CON"]
        .into_iter()
        .filter(|n| present.iter().any(|p| p == n))
        .collect();
    assert!(names.contains(&"ok.txt"), "ok.txt missing: {present:?}");
    assert!(names.len() >= 2, "no unsafe name was created: {present:?}");
    for rules in [true, false] {
        let mut c = cfg(&fx.root(), false);
        c.safe_root = Some(std::fs::canonicalize(&root).unwrap());
        c.win_rules = rules;
        let Outcome::Reply(r) = render_listing(&c, &fx.root(), "/").await else {
            panic!("not Reply")
        };
        let Body::Bytes(b) = r.body else {
            panic!("not Bytes")
        };
        let h = String::from_utf8(b).unwrap();
        for n in &names {
            let shown = h.contains(&format!(">{n}<"));
            assert_eq!(shown, !rules || *n == "ok.txt", "rules={rules} name={n}");
        }
    }
}
